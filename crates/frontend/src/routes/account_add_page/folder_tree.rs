use super::dtos::{NewFolderDto, RemoteFolderDto};

/// A single folder in the [`FolderTree`]. Parent and children are referenced by
/// index into the tree's flat `nodes` vector rather than by owned pointers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FolderNode {
    pub path: String,
    pub name: String,
    pub special_use: Option<String>,
    pub no_select: bool,
    pub depth: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub selected: bool,
}

/// An index-based flat tree of folders. Nodes live in a single `nodes` vector
/// and reference their parent/children by index, avoiding borrow-checker
/// friction.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FolderTree {
    pub nodes: Vec<FolderNode>,
}

fn split_path<'a>(path: &'a str, delim: &str) -> Vec<&'a str> {
    if delim.is_empty() { vec![path] } else { path.split(delim).collect() }
}

impl FolderTree {
    /// Builds a parent-before-child ordered tree. Parent linkage is derived
    /// from the path prefix using each entry's delimiter (falling back to
    /// `"/"`). Folders whose parent is not in the list become roots
    /// (defensive — servers can omit container rows).
    pub(crate) fn build(folders: Vec<RemoteFolderDto>) -> Self {
        let mut indexed: Vec<(usize, RemoteFolderDto)> = folders.into_iter().enumerate().collect();
        indexed.sort_by_key(|(i, f)| {
            let delim = f.delimiter.clone().unwrap_or_else(|| "/".into());
            (split_path(&f.path, &delim).len(), *i)
        });

        let mut nodes: Vec<FolderNode> = Vec::with_capacity(indexed.len());
        let mut path_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        for (_, f) in indexed {
            let delim = f.delimiter.clone().unwrap_or_else(|| "/".into());
            let segs = split_path(&f.path, &delim);
            let name = segs.last().copied().unwrap_or(&f.path).to_string();
            let depth = segs.len().saturating_sub(1);
            let parent = if segs.len() > 1 {
                let parent_path = segs[..segs.len() - 1].join(&delim);
                path_to_idx.get(&parent_path).copied()
            } else {
                None
            };
            let idx = nodes.len();
            if let Some(p) = parent {
                nodes[p].children.push(idx);
            }
            path_to_idx.insert(f.path.clone(), idx);
            nodes.push(FolderNode {
                path: f.path,
                name,
                special_use: f.special_use,
                no_select: f.no_select,
                depth,
                parent,
                children: Vec::new(),
                selected: false,
            });
        }
        Self { nodes }
    }

    fn descendants(&self, idx: usize, out: &mut Vec<usize>) {
        for &c in &self.nodes[idx].children {
            out.push(c);
            self.descendants(c, out);
        }
    }

    /// Set a node and all its descendants to `value`.
    pub(crate) fn set_subtree(&mut self, idx: usize, value: bool) {
        self.nodes[idx].selected = value;
        let mut ds = Vec::new();
        self.descendants(idx, &mut ds);
        for d in ds {
            self.nodes[d].selected = value;
        }
    }

    /// Indeterminate iff some-but-not-all selectable descendants are selected.
    pub(crate) fn is_indeterminate(&self, idx: usize) -> bool {
        let mut ds = Vec::new();
        self.descendants(idx, &mut ds);
        let selectable: Vec<usize> = ds.into_iter().filter(|&d| !self.nodes[d].no_select).collect();
        if selectable.is_empty() {
            return false;
        }
        let sel = selectable.iter().filter(|&&d| self.nodes[d].selected).count();
        sel > 0 && sel < selectable.len()
    }

    pub(crate) fn select_all(&mut self, value: bool) {
        for n in &mut self.nodes {
            n.selected = value;
        }
    }

    /// Default-select the inbox (special-use first, then INBOX-by-name).
    pub(crate) fn default_select_inbox(&mut self) {
        if let Some(i) = self.nodes.iter().position(|n| n.special_use.as_deref() == Some("inbox")) {
            self.nodes[i].selected = true;
            return;
        }
        if let Some(i) = self.nodes.iter().position(|n| n.name.eq_ignore_ascii_case("INBOX")) {
            self.nodes[i].selected = true;
        }
    }

    pub(crate) fn selected_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.selected && !n.no_select).count()
    }

    /// Returns the selected, selectable folders as the create payload.
    /// `\Noselect` folders are filtered out per spec §6 (they are
    /// containers, not mailboxes).
    pub(crate) fn selected_new_folders(&self) -> Vec<NewFolderDto> {
        self.nodes
            .iter()
            .filter(|n| n.selected && !n.no_select)
            .map(|n| NewFolderDto {
                path: n.path.clone(),
                special_use: n.special_use.clone(),
                no_select: false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, su: Option<&str>, no_select: bool) -> RemoteFolderDto {
        RemoteFolderDto {
            path: path.into(),
            special_use: su.map(str::to_string),
            has_children: false,
            no_select,
            delimiter: Some("/".into()),
        }
    }

    fn gmail_tree() -> FolderTree {
        FolderTree::build(vec![
            f("INBOX", Some("inbox"), false),
            f("[Gmail]", None, true), // \Noselect container
            f("[Gmail]/All Mail", Some("all"), false),
            f("[Gmail]/Sent Mail", Some("sent"), false),
        ])
    }

    #[test]
    fn builds_parent_child_links() {
        let t = gmail_tree();
        let gmail = t.nodes.iter().position(|n| n.path == "[Gmail]").unwrap();
        assert_eq!(t.nodes[gmail].children.len(), 2);
        let all = t.nodes.iter().find(|n| n.path == "[Gmail]/All Mail").unwrap();
        assert_eq!(all.name, "All Mail");
        assert_eq!(all.depth, 1);
    }

    #[test]
    fn cascade_check_and_uncheck() {
        let mut t = gmail_tree();
        let gmail = t.nodes.iter().position(|n| n.path == "[Gmail]").unwrap();
        t.set_subtree(gmail, true);
        assert!(t.nodes.iter().filter(|n| n.path.starts_with("[Gmail]")).all(|n| n.selected));
        t.set_subtree(gmail, false);
        assert!(t.nodes.iter().filter(|n| n.path.starts_with("[Gmail]")).all(|n| !n.selected));
    }

    #[test]
    fn indeterminate_when_partial() {
        let mut t = gmail_tree();
        let all = t.nodes.iter().position(|n| n.path == "[Gmail]/All Mail").unwrap();
        let gmail = t.nodes.iter().position(|n| n.path == "[Gmail]").unwrap();
        t.nodes[all].selected = true;
        assert!(t.is_indeterminate(gmail));
        let sent = t.nodes.iter().position(|n| n.path == "[Gmail]/Sent Mail").unwrap();
        t.nodes[sent].selected = true;
        assert!(!t.is_indeterminate(gmail));
    }

    #[test]
    fn default_inbox_by_special_use() {
        let mut t = gmail_tree();
        t.default_select_inbox();
        assert!(t.nodes.iter().find(|n| n.path == "INBOX").unwrap().selected);
    }

    #[test]
    fn default_inbox_by_name_fallback() {
        let mut t = FolderTree::build(vec![f("INBOX", None, false), f("Archive", None, false)]);
        t.default_select_inbox();
        assert!(t.nodes.iter().find(|n| n.path == "INBOX").unwrap().selected);
    }

    #[test]
    fn noselect_excluded_from_payload_but_cascades() {
        let mut t = gmail_tree();
        let gmail = t.nodes.iter().position(|n| n.path == "[Gmail]").unwrap();
        t.set_subtree(gmail, true);
        let payload = t.selected_new_folders();
        assert!(payload.iter().all(|p| !p.no_select));
        assert!(!payload.iter().any(|p| p.path == "[Gmail]"));
        assert!(payload.iter().any(|p| p.path == "[Gmail]/All Mail"));
        assert_eq!(t.selected_count(), 2);
    }

    #[test]
    fn select_all_then_none() {
        let mut t = gmail_tree();
        t.select_all(true);
        assert_eq!(t.selected_count(), 3);
        t.select_all(false);
        assert_eq!(t.selected_count(), 0);
    }
}

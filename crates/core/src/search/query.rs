//! Hinted search query grammar: a pure tokenizer + parser producing a [`Query`]
//! AST.
//!
//! The grammar is deliberately lenient — parsing is **total** and never returns
//! an error. Any malformed fragment (unknown field, invalid date, unterminated
//! quote, bad attachment value) degrades gracefully to a bare full-text term
//! rather than failing the whole query.
//!
//! ```text
//! Query   := Clause*
//! Clause  := ['!'] (FieldHint | BareTerm)
//! FieldHint := field ':' Value
//! Value   := QuotedPhrase | Word
//! ```
//!
//! The resulting AST carries no query-engine semantics — implied AND between
//! clauses, field resolution (account/folder names), and full-text matching are
//! all the consumer's responsibility (see the Tantivy adapter).
//!
//! Note: quoting does NOT suppress field-hint interpretation of an inner colon.
//! Because the tokenizer strips quotes before the field/value split, a quoted
//! phrase like `"from: the list"` still parses as a `from:` hint. This is an
//! accepted v1 tradeoff of the tokenize-then-reparse design.

use chrono::NaiveDate;

/// A parsed search query: an ordered list of clauses combined with implied AND.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub clauses: Vec<Clause>,
}

/// A single clause, optionally negated with a leading `!`.
#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    pub negated: bool,
    pub term: Term,
}

/// The matchable content of a clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// Full-text search over subject + body.
    Bare(String),
    /// Field-scoped text match (subject / from / to).
    Text { field: TextField, value: String },
    /// Date comparison against the message date.
    Date { bound: DateBound, date: NaiveDate },
    /// `true` = has attachments, `false` = has none.
    Attachments(bool),
    /// Raw account name, resolved later within the user scope.
    Account(String),
    /// Raw folder name, resolved later (DB post-filter).
    Folder(String),
}

/// Which text field a [`Term::Text`] matches against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextField {
    Subject,
    From,
    To,
}

/// The comparison bound for a [`Term::Date`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateBound {
    Before,
    After,
    On,
}

/// Parse a raw query string into a [`Query`].
///
/// This is total: it always succeeds. Empty or whitespace-only input yields a
/// query with zero clauses.
pub fn parse(input: &str) -> Query {
    let clauses = tokenize(input).into_iter().filter_map(|token| parse_token(&token)).collect();
    Query { clauses }
}

/// Split input into logical tokens.
///
/// Tokens are whitespace-separated, except that a double quote groups a
/// multi-word run into a single token with its inner spaces preserved. The
/// surrounding quote characters are stripped. An unterminated quote extends to
/// the end of the input, degrading to a single token rather than failing.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    // Tracks whether `current` holds a started token, so that an empty quoted
    // run (`""`) or a lone `!` still forms a token, while runs of whitespace
    // do not emit empty tokens.
    let mut started = false;

    for ch in input.chars() {
        if in_quotes {
            if ch == '"' {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
            started = true;
        } else if ch.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
            started = true;
        }
    }

    if started {
        tokens.push(current);
    }

    tokens
}

/// Parse one token into a clause, or `None` if it carries no content (a lone
/// `!` or an empty quoted run).
fn parse_token(token: &str) -> Option<Clause> {
    let (negated, rest) = match token.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, token),
    };

    if rest.is_empty() {
        return None;
    }

    Some(Clause {
        negated,
        term: parse_term(rest),
    })
}

/// Parse the content portion of a token (after any `!`) into a [`Term`].
fn parse_term(rest: &str) -> Term {
    if let Some((field, value)) = rest.split_once(':')
        && let Some(term) = parse_field(field, value)
    {
        return term;
    }
    // No colon, an unknown field, or an invalid value: degrade to a bare term
    // covering the whole content (colon included, if any).
    Term::Bare(rest.to_string())
}

/// Map a known `field:value` pair to its [`Term`]. Returns `None` for unknown
/// fields, empty values, or invalid values so the caller can degrade to a bare
/// term.
fn parse_field(field: &str, value: &str) -> Option<Term> {
    // An empty value (e.g. `subject:` or `account:`) can never match usefully
    // downstream, so degrade it uniformly to a bare term like the date and
    // attachment fields already do.
    if value.is_empty() {
        return None;
    }

    match field {
        "subject" => Some(Term::Text {
            field: TextField::Subject,
            value: value.to_string(),
        }),
        "from" => Some(Term::Text {
            field: TextField::From,
            value: value.to_string(),
        }),
        "to" => Some(Term::Text {
            field: TextField::To,
            value: value.to_string(),
        }),
        "account" => Some(Term::Account(value.to_string())),
        "folder" => Some(Term::Folder(value.to_string())),
        "before" => parse_date(value).map(|date| Term::Date {
            bound: DateBound::Before,
            date,
        }),
        "after" => parse_date(value).map(|date| Term::Date { bound: DateBound::After, date }),
        "date" => parse_date(value).map(|date| Term::Date { bound: DateBound::On, date }),
        "attachments" if value.eq_ignore_ascii_case("none") => Some(Term::Attachments(false)),
        "attachments" if value.eq_ignore_ascii_case("some") => Some(Term::Attachments(true)),
        "has" if value.eq_ignore_ascii_case("attachment") => Some(Term::Attachments(true)),
        _ => None,
    }
}

/// Parse a `YYYY-MM-DD` date, returning `None` on any parse failure.
fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn empty_input_yields_no_clauses() {
        assert_eq!(parse(""), Query { clauses: vec![] });
    }

    #[test]
    fn whitespace_only_input_yields_no_clauses() {
        assert_eq!(parse("   \t \n  "), Query { clauses: vec![] });
    }

    #[test]
    fn single_bare_term() {
        assert_eq!(
            parse("coffee").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("coffee".to_string()),
            }],
        );
    }

    #[test]
    fn multiple_bare_terms_preserve_order() {
        assert_eq!(
            parse("alpha beta gamma").clauses,
            vec![
                Clause {
                    negated: false,
                    term: Term::Bare("alpha".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Bare("beta".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Bare("gamma".to_string())
                },
            ],
        );
    }

    #[test]
    fn subject_field() {
        assert_eq!(
            parse("subject:Amazon").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::Subject,
                    value: "Amazon".to_string()
                },
            }],
        );
    }

    #[test]
    fn from_field() {
        assert_eq!(
            parse("from:alice").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::From,
                    value: "alice".to_string()
                },
            }],
        );
    }

    #[test]
    fn to_field() {
        assert_eq!(
            parse("to:bob").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::To,
                    value: "bob".to_string()
                },
            }],
        );
    }

    #[test]
    fn account_field() {
        assert_eq!(
            parse("account:Fastmail").clauses,
            vec![Clause {
                negated: false,
                term: Term::Account("Fastmail".to_string()),
            }],
        );
    }

    #[test]
    fn folder_field() {
        assert_eq!(
            parse("folder:orders").clauses,
            vec![Clause {
                negated: false,
                term: Term::Folder("orders".to_string()),
            }],
        );
    }

    #[test]
    fn before_date() {
        assert_eq!(
            parse("before:2024-01-15").clauses,
            vec![Clause {
                negated: false,
                term: Term::Date {
                    bound: DateBound::Before,
                    date: date(2024, 1, 15)
                },
            }],
        );
    }

    #[test]
    fn after_date() {
        assert_eq!(
            parse("after:2024-06-30").clauses,
            vec![Clause {
                negated: false,
                term: Term::Date {
                    bound: DateBound::After,
                    date: date(2024, 6, 30)
                },
            }],
        );
    }

    #[test]
    fn on_date() {
        assert_eq!(
            parse("date:2024-12-25").clauses,
            vec![Clause {
                negated: false,
                term: Term::Date {
                    bound: DateBound::On,
                    date: date(2024, 12, 25)
                },
            }],
        );
    }

    #[test]
    fn invalid_date_degrades_to_bare() {
        assert_eq!(
            parse("before:notadate").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("before:notadate".to_string()),
            }],
        );
    }

    #[test]
    fn attachments_none() {
        assert_eq!(
            parse("attachments:none").clauses,
            vec![Clause {
                negated: false,
                term: Term::Attachments(false),
            }],
        );
    }

    #[test]
    fn attachments_some() {
        assert_eq!(
            parse("attachments:some").clauses,
            vec![Clause {
                negated: false,
                term: Term::Attachments(true),
            }],
        );
    }

    #[test]
    fn attachments_value_is_case_insensitive() {
        assert_eq!(
            parse("attachments:NONE").clauses,
            vec![Clause {
                negated: false,
                term: Term::Attachments(false),
            }],
        );
    }

    #[test]
    fn attachments_invalid_value_degrades_to_bare() {
        assert_eq!(
            parse("attachments:maybe").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("attachments:maybe".to_string()),
            }],
        );
    }

    #[test]
    fn has_attachment() {
        assert_eq!(
            parse("has:attachment").clauses,
            vec![Clause {
                negated: false,
                term: Term::Attachments(true),
            }],
        );
    }

    #[test]
    fn has_value_is_case_insensitive() {
        assert_eq!(
            parse("has:Attachment").clauses,
            vec![Clause {
                negated: false,
                term: Term::Attachments(true),
            }],
        );
    }

    #[test]
    fn has_invalid_value_degrades_to_bare() {
        assert_eq!(
            parse("has:wings").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("has:wings".to_string()),
            }],
        );
    }

    #[test]
    fn unknown_field_degrades_to_bare_including_colon() {
        assert_eq!(
            parse("foo:bar").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("foo:bar".to_string()),
            }],
        );
    }

    #[test]
    fn negated_bare_term() {
        assert_eq!(
            parse("!filters").clauses,
            vec![Clause {
                negated: true,
                term: Term::Bare("filters".to_string()),
            }],
        );
    }

    #[test]
    fn negated_field() {
        assert_eq!(
            parse("!folder:spam").clauses,
            vec![Clause {
                negated: true,
                term: Term::Folder("spam".to_string()),
            }],
        );
    }

    #[test]
    fn lone_bang_is_skipped() {
        assert_eq!(parse("!").clauses, vec![]);
    }

    #[test]
    fn detached_bang_does_not_negate_following_word() {
        // A `!` separated by whitespace is its own (skipped) token and does not
        // negate the next word.
        assert_eq!(
            parse("! filters").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("filters".to_string()),
            }],
        );
    }

    #[test]
    fn quoted_phrase_field_value() {
        assert_eq!(
            parse("subject:\"order confirmation\"").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::Subject,
                    value: "order confirmation".to_string()
                },
            }],
        );
    }

    #[test]
    fn quoted_bare_phrase() {
        assert_eq!(
            parse("\"two words\"").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("two words".to_string()),
            }],
        );
    }

    #[test]
    fn unterminated_quote_degrades_to_single_token() {
        assert_eq!(
            parse("subject:\"order conf").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::Subject,
                    value: "order conf".to_string()
                },
            }],
        );
    }

    #[test]
    fn unterminated_bare_quote_degrades_to_single_token() {
        assert_eq!(
            parse("hello \"two words unclosed").clauses,
            vec![
                Clause {
                    negated: false,
                    term: Term::Bare("hello".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Bare("two words unclosed".to_string())
                },
            ],
        );
    }

    #[test]
    fn negation_applies_to_quoted_field_value() {
        // Edge-case decision: a `!` prefix on a field with a quoted value
        // negates the whole field clause (spaces preserved from the quote).
        assert_eq!(
            parse("!subject:\"a b\"").clauses,
            vec![Clause {
                negated: true,
                term: Term::Text {
                    field: TextField::Subject,
                    value: "a b".to_string()
                },
            }],
        );
    }

    #[test]
    fn empty_text_field_value_degrades_to_bare() {
        assert_eq!(
            parse("subject:").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("subject:".to_string()),
            }],
        );
    }

    #[test]
    fn empty_account_field_value_degrades_to_bare() {
        assert_eq!(
            parse("account:").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("account:".to_string()),
            }],
        );
    }

    #[test]
    fn empty_date_field_value_degrades_to_bare() {
        assert_eq!(
            parse("before:").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("before:".to_string()),
            }],
        );
    }

    #[test]
    fn empty_quoted_run_in_isolation_yields_no_clause() {
        assert_eq!(parse("\"\"").clauses, vec![]);
    }

    #[test]
    fn colon_inside_quoted_value_stays_in_value() {
        assert_eq!(
            parse("subject:\"a:b\"").clauses,
            vec![Clause {
                negated: false,
                term: Term::Text {
                    field: TextField::Subject,
                    value: "a:b".to_string()
                },
            }],
        );
    }

    #[test]
    fn multiple_colons_split_on_first_then_degrade_to_bare() {
        // `a` is not a known field, so the whole token degrades to a bare term.
        assert_eq!(
            parse("a:b:c").clauses,
            vec![Clause {
                negated: false,
                term: Term::Bare("a:b:c".to_string()),
            }],
        );
    }

    #[test]
    fn double_bang_negates_and_keeps_second_bang_in_bare() {
        // Only the leading `!` is consumed as negation; the rest is bare.
        assert_eq!(
            parse("!!foo").clauses,
            vec![Clause {
                negated: true,
                term: Term::Bare("!foo".to_string()),
            }],
        );
    }

    #[test]
    fn worked_example_full_ast() {
        let query = parse("account:Fastmail subject:Amazon folder:orders coffee !filters attachments:none");
        assert_eq!(
            query.clauses,
            vec![
                Clause {
                    negated: false,
                    term: Term::Account("Fastmail".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Text {
                        field: TextField::Subject,
                        value: "Amazon".to_string()
                    }
                },
                Clause {
                    negated: false,
                    term: Term::Folder("orders".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Bare("coffee".to_string())
                },
                Clause {
                    negated: true,
                    term: Term::Bare("filters".to_string())
                },
                Clause {
                    negated: false,
                    term: Term::Attachments(false)
                },
            ],
        );
    }
}

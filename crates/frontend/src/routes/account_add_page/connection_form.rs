use dioxus::prelude::*;

/// Always-visible IMAP connection section. All field state is owned by the page
/// (Task 8) and passed in as signals; this component only renders inputs bound
/// to those signals plus a probe button, emitting `on_probe` when clicked.
///
/// Port auto-fills from the encryption choice (Tls→993, StartTls→143) until the
/// user edits the port, at which point `port_touched` latches and auto-fill
/// stops.
#[component]
pub(crate) fn ConnectionForm(
    display_name: Signal<String>,
    host: Signal<String>,
    port: Signal<String>,
    tls: Signal<String>,
    email: Signal<String>,
    password: Signal<String>,
    port_touched: Signal<bool>,
    probing: bool,
    on_probe: EventHandler<()>,
) -> Element {
    // Auto-fill port from the encryption choice unless the user has touched it.
    use_effect(move || {
        if !port_touched() {
            let default = if tls() == "StartTls" { "143" } else { "993" };
            port.set(default.to_string());
        }
    });

    rsx! {
        div { class: "space-y-4",
            // Display name
            div {
                label {
                    class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                    r#for: "acc-display-name",
                    "Display name"
                }
                input {
                    id: "acc-display-name",
                    r#type: "text",
                    class: input_class(),
                    placeholder: "e.g. Personal Gmail",
                    value: display_name,
                    disabled: probing,
                    oninput: move |e| display_name.set(e.value()),
                }
            }

            // Host + Port + Encryption row
            div { class: "grid grid-cols-6 gap-3",
                div { class: "col-span-3",
                    label {
                        class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                        r#for: "acc-host",
                        "IMAP server"
                    }
                    input {
                        id: "acc-host",
                        r#type: "text",
                        class: input_class(),
                        placeholder: "imap.example.com",
                        value: host,
                        disabled: probing,
                        oninput: move |e| host.set(e.value()),
                    }
                }
                div { class: "col-span-1",
                    label {
                        class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                        r#for: "acc-port",
                        "Port"
                    }
                    input {
                        id: "acc-port",
                        r#type: "number",
                        class: input_class(),
                        value: port,
                        disabled: probing,
                        oninput: move |e| {
                            port_touched.set(true);
                            port.set(e.value());
                        },
                    }
                }
                div { class: "col-span-2",
                    label {
                        class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                        r#for: "acc-encryption",
                        "Encryption"
                    }
                    select {
                        id: "acc-encryption",
                        class: input_class(),
                        disabled: probing,
                        onchange: move |e| tls.set(e.value()),
                        option { value: "Tls", selected: tls() == "Tls", "Implicit TLS" }
                        option { value: "StartTls", selected: tls() == "StartTls", "STARTTLS" }
                    }
                }
            }

            // Email
            div {
                label {
                    class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                    r#for: "acc-email",
                    "Email address"
                }
                input {
                    id: "acc-email",
                    r#type: "text",
                    class: input_class(),
                    placeholder: "you@example.com",
                    value: email,
                    disabled: probing,
                    oninput: move |e| email.set(e.value()),
                }
                p { class: "text-xs text-gray-400 dark:text-slate-500 mt-1", "Also your IMAP login." }
            }

            // Password
            div {
                label {
                    class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1",
                    r#for: "acc-password",
                    "Password"
                }
                input {
                    id: "acc-password",
                    r#type: "password",
                    class: input_class(),
                    value: password,
                    disabled: probing,
                    oninput: move |e| password.set(e.value()),
                }
            }

            // Probe button — type=button so it never submits the surrounding form.
            button {
                class: "w-full py-2 px-4 bg-indigo-600 hover:bg-indigo-700 disabled:bg-indigo-400 text-white font-semibold rounded-lg transition-colors",
                r#type: "button",
                disabled: probing,
                onclick: move |_| on_probe.call(()),
                if probing { "Testing…" } else { "Test connection & load folders" }
            }
        }
    }
}

fn input_class() -> &'static str {
    "w-full px-3 py-2 border border-gray-300 dark:border-slate-600 rounded-lg bg-white dark:bg-slate-700 text-gray-900 dark:text-slate-100 \
     focus:outline-hidden focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500"
}

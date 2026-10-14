use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

actions!(cooper, [FocusCapture, FocusSearch, CancelEdit]);

#[derive(Clone, Deserialize, Serialize)]
struct Capture {
    id: u64,
    text: String,
    complete: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Open,
    Done,
}

enum Composer {
    Capture,
    Edit { id: u64, capture_draft: String },
}

struct Cooper {
    items: Vec<Capture>,
    capture: Entity<InputState>,
    search: Entity<InputState>,
    composer: Composer,
    filter: Filter,
    path: PathBuf,
    load_failed: bool,
    error: Option<String>,
    status: String,
    _subscriptions: Vec<Subscription>,
}

impl Cooper {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/captures.json");
        let loaded = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Vec<Capture>>(&bytes).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
        }
        .and_then(|items| {
            let mut ids = std::collections::HashSet::new();
            if items
                .iter()
                .any(|item| item.text.trim().is_empty() || !ids.insert(item.id))
            {
                Err("The file contains blank captures or duplicate IDs.".to_owned())
            } else {
                Ok(items)
            }
        });
        let load_failed = loaded.is_err();
        let error = loaded.as_ref().err().map(|e| format!("Cannot load captures. {e} Your file has been left untouched. Repair data/captures.json and restart."));
        let items = loaded.unwrap_or_default();
        let status = format!("{} captures loaded", items.len());
        let capture = cx.new(|cx| InputState::new(window, cx).placeholder("Capture a thought..."));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search captures..."));
        let submit = cx.subscribe_in(&capture, window, |_, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                cx.defer_in(window, |this, window, cx| this.submit(window, cx));
            }
        });
        let query = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        capture.update(cx, |input, cx| input.focus(window, cx));
        Self {
            items,
            capture,
            search,
            composer: Composer::Capture,
            filter: Filter::All,
            path,
            load_failed,
            error,
            status,
            _subscriptions: vec![submit, query],
        }
    }

    fn persist(&mut self, items: Vec<Capture>, message: &str, cx: &mut Context<Self>) -> bool {
        if self.load_failed {
            return false;
        }
        let saved = (|| -> Result<(), Box<dyn std::error::Error>> {
            let bytes = serde_json::to_vec_pretty(&items)?;
            fs::create_dir_all(self.path.parent().expect("data directory"))?;
            let temporary = self.path.with_extension("json.tmp");
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(temporary, &self.path)?;
            Ok(())
        })();
        match saved {
            Ok(()) => {
                self.items = items;
                self.error = None;
                self.status = message.to_owned();
                cx.notify();
                true
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not save. {e} Your change has not been applied. Try again after fixing the file access."
                ));
                cx.notify();
                false
            }
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.capture.read(cx).value().trim().to_owned();
        if text.is_empty() || self.load_failed {
            return;
        }
        let mut items = self.items.clone();
        let message = match &self.composer {
            Composer::Capture => {
                let Some(id) = items
                    .iter()
                    .map(|item| item.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                else {
                    self.error = Some("Capture ID limit reached.".to_owned());
                    cx.notify();
                    return;
                };
                items.insert(
                    0,
                    Capture {
                        id,
                        text,
                        complete: false,
                    },
                );
                "Capture saved"
            }
            Composer::Edit { id, .. } => {
                let Some(item) = items.iter_mut().find(|item| item.id == *id) else {
                    return;
                };
                item.text = text;
                "Edit saved"
            }
        };
        if self.persist(items, message, cx) {
            let previous = std::mem::replace(&mut self.composer, Composer::Capture);
            let draft = match previous {
                Composer::Capture => {
                    self.filter = Filter::All;
                    self.search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    String::new()
                }
                Composer::Edit { capture_draft, .. } => capture_draft,
            };
            self.capture.update(cx, |input, cx| {
                input.set_value(draft, window, cx);
                input.focus(window, cx);
            });
        }
    }

    fn edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.load_failed {
            return;
        }
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return;
        };
        let text = item.text.clone();
        let capture_draft = match std::mem::replace(&mut self.composer, Composer::Capture) {
            Composer::Capture => self.capture.read(cx).value().to_string(),
            Composer::Edit { capture_draft, .. } => capture_draft,
        };
        self.composer = Composer::Edit { id, capture_draft };
        self.capture.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, _: &CancelEdit, window: &mut Window, cx: &mut Context<Self>) {
        if let Composer::Edit { capture_draft, .. } =
            std::mem::replace(&mut self.composer, Composer::Capture)
        {
            self.capture.update(cx, |input, cx| {
                input.set_value(capture_draft, window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        }
    }

    fn focus_capture(&mut self, _: &FocusCapture, window: &mut Window, cx: &mut Context<Self>) {
        self.capture.update(cx, |input, cx| input.focus(window, cx));
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.focus(window, cx));
    }
}

impl Render for Cooper {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let visible: Vec<_> = self
            .items
            .iter()
            .filter(|item| {
                (match self.filter {
                    Filter::All => true,
                    Filter::Open => !item.complete,
                    Filter::Done => item.complete,
                }) && item.text.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let editing = matches!(self.composer, Composer::Edit { .. });
        let open = self.items.iter().filter(|item| !item.complete).count();
        let mut filters = div().flex().gap_1();
        for (index, (filter, label)) in [
            (Filter::All, "All"),
            (Filter::Open, "Open"),
            (Filter::Done, "Done"),
        ]
        .into_iter()
        .enumerate()
        {
            filters = filters.child(
                Button::new(("filter", index))
                    .small()
                    .label(label)
                    .selected(self.filter == filter)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = filter;
                        cx.notify();
                    })),
            );
        }
        let mut cards = div()
            .id("capture-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .overflow_y_scroll();
        if visible.is_empty() {
            cards = cards.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_6()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(if self.items.is_empty() {
                        "A place for your next thought."
                    } else {
                        "No matching captures."
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.items.is_empty() {
                                "Type above and press Enter to get started."
                            } else {
                                "Try another search or filter."
                            }),
                    ),
            );
        }
        for item in visible {
            let id = item.id;
            let is_editing =
                matches!(self.composer, Composer::Edit { id: editing_id, .. } if editing_id == id);
            cards = cards.child(div().id(("card", id)).flex().flex_col().flex_shrink_0().gap_3().p_3().rounded_lg()
                .bg(cx.theme().secondary).border_1()
                .border_color(if is_editing { cx.theme().primary } else { cx.theme().border })
                .child(div().text_color(if item.complete { cx.theme().muted_foreground } else { cx.theme().foreground }).child(item.text))
                .child(div().flex().items_center().justify_between().gap_2()
                    .child(Checkbox::new(("complete", id)).label(if item.complete { "Completed" } else { "Mark complete" })
                        .checked(item.complete).disabled(self.load_failed)
                        .on_change(cx.listener(move |this, complete, _, cx| {
                            let mut items = this.items.clone();
                            if let Some(item) = items.iter_mut().find(|item| item.id == id) { item.complete = *complete; }
                            this.persist(items, "Completion saved", cx);
                        })))
                    .child(div().flex().gap_1()
                        .child(Button::new(("edit", id)).ghost().small().label("Edit").disabled(self.load_failed)
                            .on_click(cx.listener(move |this, _, window, cx| this.edit(id, window, cx))))
                        .child(Button::new(("delete", id)).ghost().small().label("Delete").disabled(self.load_failed)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let items = this.items.iter().filter(|item| item.id != id).cloned().collect();
                                if this.persist(items, "Capture deleted", cx)
                                    && matches!(this.composer, Composer::Edit { id: editing_id, .. } if editing_id == id) {
                                    this.cancel_edit(&CancelEdit, window, cx);
                                }
                            }))))));
        }
        let mut composer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.capture).disabled(self.load_failed)),
            )
            .child(
                Button::new("add")
                    .primary()
                    .label(if editing { "Save" } else { "Add" })
                    .disabled(self.load_failed)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        if editing {
            composer = composer.child(Button::new("cancel").ghost().label("Cancel").on_click(
                cx.listener(|this, _, window, cx| this.cancel_edit(&CancelEdit, window, cx)),
            ));
        }
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .key_context("Cooper")
            .on_action(cx.listener(Self::focus_capture))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::cancel_edit))
            .font_family("Segoe UI")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Cooper"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{open} open")),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if editing {
                        "Edit capture · Enter to save · Escape to cancel"
                    } else {
                        "Capture now. Come back when you're ready."
                    }),
            )
            .child(composer)
            .child(div().child(Input::new(&self.search).cleanable(true)))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(filters)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} of {}",
                                self.items
                                    .iter()
                                    .filter(|item| {
                                        (self.filter == Filter::All
                                            || (self.filter == Filter::Done) == item.complete)
                                            && item.text.to_lowercase().contains(&query)
                                    })
                                    .count(),
                                self.items.len()
                            )),
                    ),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .text_sm()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(0x44232b))
                    .text_color(rgb(0xffb4c1))
                    .child(error.clone()),
            );
        }
        content
            .child(cards)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Ctrl+N capture · Ctrl+F search"),
            )
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-n", FocusCapture, Some("Cooper")),
            KeyBinding::new("ctrl-f", FocusSearch, Some("Cooper")),
            KeyBinding::new("escape", CancelEdit, Some("Cooper")),
        ]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(520.), px(700.)), cx);
        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(420.), px(460.))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                window.set_window_title("Cooper capture prototype");
                cx.new(|cx| Cooper::new(window, cx))
            },
        )
        .expect("open Cooper window");
        cx.activate(true);
    });
}

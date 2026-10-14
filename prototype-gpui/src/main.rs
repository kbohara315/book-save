use futures::StreamExt;
use gpui_kit::assets::icon_assets;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

mod global_capture;
use global_capture::{CaptureEvent, CaptureWorker};

actions!(cooper, [FocusCapture, FocusSearch, CancelEdit]);

icon_assets!(AppAssets, [Check, Close]);

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

struct PersistRequest {
    seq: u64,
    snapshot: Vec<Capture>,
    path: PathBuf,
}

struct PersistResult {
    seq: u64,
    result: Result<(), String>,
}

fn write_snapshot(path: &PathBuf, snapshot: &[Capture]) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    fs::create_dir_all(path.parent().expect("data directory"))?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

struct PersistWorker {
    tx: Option<std::sync::mpsc::Sender<PersistRequest>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PersistWorker {
    fn new(completion: futures::channel::mpsc::UnboundedSender<PersistResult>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<PersistRequest>();
        let thread = std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                let result = write_snapshot(&req.path, &req.snapshot).map_err(|e| e.to_string());
                let _ = completion.unbounded_send(PersistResult {
                    seq: req.seq,
                    result,
                });
            }
        });
        Self {
            tx: Some(tx),
            thread: Some(thread),
        }
    }

    fn submit(&self, req: PersistRequest) -> bool {
        match &self.tx {
            Some(tx) => tx.send(req).is_ok(),
            None => false,
        }
    }

    fn shutdown(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PersistWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum PendingEffect {
    SubmitCapture,
    SubmitEdit { capture_draft: String },
    Toggle,
    Delete { deleted_id: u64 },
}

struct PendingSave {
    seq: u64,
    snapshot: Vec<Capture>,
    message: String,
    effect: PendingEffect,
}

fn lowered_for(items: &[Capture]) -> Vec<String> {
    items.iter().map(|item| item.text.to_lowercase()).collect()
}

fn filter_matches(filter: Filter, complete: bool) -> bool {
    match filter {
        Filter::All => true,
        Filter::Open => !complete,
        Filter::Done => complete,
    }
}

fn can_deliver_deferred_capture(saving: bool, load_failed: bool, editing: bool) -> bool {
    !saving && !load_failed && !editing
}

struct Cooper {
    items: Vec<Capture>,
    lowered: Vec<String>,
    capture: Entity<InputState>,
    search: Entity<InputState>,
    composer: Composer,
    filter: Filter,
    path: PathBuf,
    load_failed: bool,
    error: Option<String>,
    status: String,
    capture_status: String,
    capture_worker: CaptureWorker,
    persist_worker: PersistWorker,
    persist_next_seq: u64,
    pending: Option<PendingSave>,
    deferred_capture: Option<String>,
    _capture_events: Task<()>,
    _persist_events: Task<()>,
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
        let lowered = lowered_for(&items);
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
        let (capture_worker, events) = global_capture::start();
        let capture_events = cx.spawn_in(window, async move |this, cx| {
            let mut events = events;
            while let Some(event) = events.next().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.receive_capture(event, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let (persist_tx, persist_rx) = futures::channel::mpsc::unbounded::<PersistResult>();
        let persist_worker = PersistWorker::new(persist_tx);
        let persist_events = cx.spawn_in(window, async move |this, cx| {
            let mut persist_rx = persist_rx;
            while let Some(result) = persist_rx.next().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.on_persist_result(result, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let shutdown = cx.on_app_quit(|this, _| {
            this.capture_worker.stop();
            this.persist_worker.shutdown();
            async {}
        });
        Self {
            items,
            lowered,
            capture,
            search,
            composer: Composer::Capture,
            filter: Filter::All,
            path,
            load_failed,
            error,
            status,
            capture_status: "Starting global capture...".into(),
            capture_worker,
            persist_worker,
            persist_next_seq: 0,
            pending: None,
            deferred_capture: None,
            _capture_events: capture_events,
            _persist_events: persist_events,
            _subscriptions: vec![submit, query, shutdown],
        }
    }

    fn receive_capture(
        &mut self,
        event: CaptureEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_status = match event {
            CaptureEvent::Ready => "Select text in another app, then double-tap Left Shift.".into(),
            CaptureEvent::Failed(error) => format!("Global capture failed. {error}"),
            CaptureEvent::SelectedText(_) if self.load_failed => {
                "Global capture skipped. Resolve the load error first.".into()
            }
            CaptureEvent::SelectedText(_) if matches!(self.composer, Composer::Edit { .. }) => {
                "Global capture skipped. Finish or cancel your edit and resolve any load error first.".into()
            }
            CaptureEvent::SelectedText(text) if self.pending.is_some() => {
                self.deferred_capture = Some(text);
                "Selected text queued until the current save finishes.".into()
            }
            CaptureEvent::SelectedText(text) => {
                self.capture.update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                    input.focus(window, cx);
                });
                window.activate_window();
                "Selected text is ready. Press Enter or Add to save.".into()
            }
        };
        cx.notify();
    }

    fn deliver_deferred_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !can_deliver_deferred_capture(
            self.pending.is_some(),
            self.load_failed,
            matches!(self.composer, Composer::Edit { .. }),
        ) {
            return;
        }
        let Some(text) = self.deferred_capture.take() else {
            return;
        };
        self.capture.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        window.activate_window();
        self.capture_status = "Selected text is ready. Press Enter or Add to save.".into();
        cx.notify();
    }

    fn base_items(&self) -> Vec<Capture> {
        match &self.pending {
            Some(pending) => pending.snapshot.clone(),
            None => self.items.clone(),
        }
    }

    fn queue_save(
        &mut self,
        snapshot: Vec<Capture>,
        message: &str,
        effect: PendingEffect,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.load_failed {
            return false;
        }
        self.persist_next_seq += 1;
        let seq = self.persist_next_seq;
        let request = PersistRequest {
            seq,
            snapshot: snapshot.clone(),
            path: self.path.clone(),
        };
        self.pending = Some(PendingSave {
            seq,
            snapshot,
            message: message.to_owned(),
            effect,
        });
        self.status = "Saving...".to_owned();
        cx.notify();
        if !self.persist_worker.submit(request) {
            self.pending = None;
            self.error = Some(
                "Could not save. The background writer is unavailable. Your change has not been applied. Try again after fixing the file access."
                    .to_owned(),
            );
            cx.notify();
            return false;
        }
        true
    }

    fn on_persist_result(
        &mut self,
        result: PersistResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if result.seq != self.persist_next_seq || result.seq != pending.seq {
            self.pending = Some(pending);
            return;
        }
        match result.result {
            Ok(()) => {
                self.items = pending.snapshot;
                self.lowered = lowered_for(&self.items);
                self.error = None;
                self.status = pending.message.clone();
                match pending.effect {
                    PendingEffect::SubmitCapture => {
                        self.composer = Composer::Capture;
                        self.filter = Filter::All;
                        self.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        self.capture.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                            input.focus(window, cx);
                        });
                    }
                    PendingEffect::SubmitEdit { capture_draft } => {
                        self.composer = Composer::Capture;
                        self.capture.update(cx, |input, cx| {
                            input.set_value(capture_draft, window, cx);
                            input.focus(window, cx);
                        });
                    }
                    PendingEffect::Toggle => {}
                    PendingEffect::Delete { deleted_id } => {
                        if matches!(self.composer, Composer::Edit { id: editing_id, .. } if editing_id == deleted_id)
                        {
                            self.cancel_edit(&CancelEdit, window, cx);
                        }
                    }
                }
                self.deliver_deferred_capture(window, cx);
                cx.notify();
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not save. {e} Your change has not been applied. Try again after fixing the file access."
                ));
                if self.deferred_capture.is_some() {
                    self.capture_status =
                        "Selected text is queued until the failed save is resolved.".into();
                }
                cx.notify();
            }
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.capture.read(cx).value().trim().to_owned();
        if text.is_empty() || self.load_failed {
            return;
        }
        if self.pending.is_some() {
            self.error = Some(
                "Could not save. A previous save is still in progress. Your change has not been applied. Try again in a moment."
                    .to_owned(),
            );
            cx.notify();
            return;
        }
        let mut items = self.base_items();
        let (message, effect) = match &self.composer {
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
                ("Capture saved", PendingEffect::SubmitCapture)
            }
            Composer::Edit { id, .. } => {
                let id = *id;
                let Some(item) = items.iter_mut().find(|item| item.id == id) else {
                    return;
                };
                item.text = text;
                let capture_draft = match &self.composer {
                    Composer::Edit { capture_draft, .. } => capture_draft.clone(),
                    Composer::Capture => String::new(),
                };
                ("Edit saved", PendingEffect::SubmitEdit { capture_draft })
            }
        };
        self.queue_save(items, message, effect, cx);
    }

    fn edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.load_failed || self.pending.is_some() {
            return;
        }
        let base = self.base_items();
        let Some(item) = base.iter().find(|item| item.id == id) else {
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
        if self.pending.is_some() {
            return;
        }
        if let Composer::Edit { capture_draft, .. } =
            std::mem::replace(&mut self.composer, Composer::Capture)
        {
            self.capture.update(cx, |input, cx| {
                input.set_value(capture_draft, window, cx);
                input.focus(window, cx);
            });
            cx.notify();
            self.deliver_deferred_capture(window, cx);
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
        let mut visible_idx: Vec<usize> = Vec::new();
        let mut open = 0usize;
        for (index, (item, low)) in self.items.iter().zip(self.lowered.iter()).enumerate() {
            if !item.complete {
                open += 1;
            }
            if filter_matches(self.filter, item.complete) && low.contains(query.as_str()) {
                visible_idx.push(index);
            }
        }
        let matched = visible_idx.len();
        let total = self.items.len();
        let editing = matches!(self.composer, Composer::Edit { .. });
        let saving = self.pending.is_some();
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
        if visible_idx.is_empty() {
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
        for index in visible_idx {
            let item = &self.items[index];
            let id = item.id;
            let complete = item.complete;
            let text = item.text.clone();
            let is_editing =
                matches!(self.composer, Composer::Edit { id: editing_id, .. } if editing_id == id);
            cards = cards.child(div().id(("card", id)).flex().flex_col().flex_shrink_0().gap_3().p_3().rounded_lg()
                .bg(cx.theme().secondary).border_1()
                .border_color(if is_editing { cx.theme().primary } else { cx.theme().border })
                .child(div().text_color(if complete { cx.theme().muted_foreground } else { cx.theme().foreground }).child(text))
                .child(div().flex().items_center().justify_between().gap_2()
                    .child(Checkbox::new(("complete", id)).label(if complete { "Completed" } else { "Mark complete" })
                        .checked(complete).disabled(self.load_failed || saving)
                        .on_change(cx.listener(move |this, complete, _, cx| {
                            if this.load_failed {
                                return;
                            }
                            if this.pending.is_some() {
                                this.error = Some("Could not save. A previous save is still in progress. Your change has not been applied. Try again in a moment.".to_owned());
                                cx.notify();
                                return;
                            }
                            let mut items = this.base_items();
                            if let Some(item) = items.iter_mut().find(|item| item.id == id) { item.complete = *complete; }
                            this.queue_save(items, "Completion saved", PendingEffect::Toggle, cx);
                        })))
                    .child(div().flex().gap_1()
                        .child(Button::new(("edit", id)).ghost().small().label("Edit").disabled(self.load_failed || saving)
                            .on_click(cx.listener(move |this, _, window, cx| this.edit(id, window, cx))))
                        .child(Button::new(("delete", id)).ghost().small().label("Delete").disabled(self.load_failed || saving)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.load_failed {
                                    return;
                                }
                                if this.pending.is_some() {
                                    this.error = Some("Could not save. A previous save is still in progress. Your change has not been applied. Try again in a moment.".to_owned());
                                    cx.notify();
                                    return;
                                }
                                let items = this.base_items().into_iter().filter(|item| item.id != id).collect();
                                this.queue_save(items, "Capture deleted", PendingEffect::Delete { deleted_id: id }, cx);
                                let _ = window;
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
                    .child(Input::new(&self.capture).disabled(self.load_failed || saving)),
            )
            .child(
                Button::new("add")
                    .primary()
                    .label(if editing { "Save" } else { "Add" })
                    .disabled(self.load_failed || saving)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        if editing {
            composer =
                composer.child(
                    Button::new("cancel")
                        .ghost()
                        .label("Cancel")
                        .disabled(saving)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_edit(&CancelEdit, window, cx)
                        })),
                );
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
                            .child(format!("{matched} of {total}")),
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
                    .child(self.capture_status.clone()),
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
    application().with_assets(AppAssets).run(|cx| {
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

#[cfg(test)]
mod perf_tests {
    use super::{
        Capture, Filter, PersistRequest, PersistWorker, can_deliver_deferred_capture,
        filter_matches, lowered_for, write_snapshot,
    };
    use std::collections::HashSet;

    fn sample_items() -> Vec<Capture> {
        vec![
            Capture {
                id: 2,
                text: "Plan the Cooper capture PANEL".into(),
                complete: true,
            },
            Capture {
                id: 1,
                text: "Read the capture notes".into(),
                complete: false,
            },
        ]
    }

    #[test]
    fn filter_search_preserves_order_and_case_insensitive_match() {
        let items = sample_items();
        let lowered = lowered_for(&items);
        assert_eq!(lowered.len(), items.len());
        let query = "panel".to_lowercase();
        let mut visible = Vec::new();
        for (item, low) in items.iter().zip(lowered.iter()) {
            if filter_matches(Filter::All, item.complete) && low.contains(query.as_str()) {
                visible.push(item.id);
            }
        }
        assert_eq!(visible, vec![2]);
        let query = "PANEL".to_lowercase();
        let mut visible = Vec::new();
        for (item, low) in items.iter().zip(lowered.iter()) {
            if filter_matches(Filter::All, item.complete) && low.contains(query.as_str()) {
                visible.push(item.id);
            }
        }
        assert_eq!(visible, vec![2]);
    }

    #[test]
    fn filter_open_done_split() {
        let items = sample_items();
        let lowered = lowered_for(&items);
        let query = String::new();
        let open: Vec<u64> = items
            .iter()
            .zip(lowered.iter())
            .filter(|(item, low)| {
                filter_matches(Filter::Open, item.complete) && low.contains(query.as_str())
            })
            .map(|(item, _)| item.id)
            .collect();
        assert_eq!(open, vec![1]);
        let done: Vec<u64> = items
            .iter()
            .zip(lowered.iter())
            .filter(|(item, low)| {
                filter_matches(Filter::Done, item.complete) && low.contains(query.as_str())
            })
            .map(|(item, _)| item.id)
            .collect();
        assert_eq!(done, vec![2]);
    }

    #[test]
    fn single_pass_count_matches_visible() {
        let items = sample_items();
        let lowered = lowered_for(&items);
        let query = "".to_lowercase();
        let mut visible = 0usize;
        let mut open = 0usize;
        for (item, low) in items.iter().zip(lowered.iter()) {
            if !item.complete {
                open += 1;
            }
            if filter_matches(Filter::All, item.complete) && low.contains(query.as_str()) {
                visible += 1;
            }
        }
        assert_eq!(visible, 2);
        assert_eq!(open, 1);
    }

    #[test]
    fn persist_bytes_match_pretty_schema_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "cooper-perf-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("captures.json");
        let items = sample_items();
        write_snapshot(&path, &items).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let bytes = std::fs::read(&path).unwrap();
        let expected = serde_json::to_vec_pretty(&items).unwrap();
        assert_eq!(bytes, expected);
        let back: Vec<Capture> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        let mut ids = HashSet::new();
        assert!(
            !back
                .iter()
                .any(|item| item.text.trim().is_empty() || !ids.insert(item.id))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn persist_replacement_is_atomic_no_tmp_left() {
        let dir = std::env::temp_dir().join(format!(
            "cooper-perf-atomic-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("captures.json");
        let first = vec![Capture {
            id: 1,
            text: "first".into(),
            complete: false,
        }];
        let second = vec![
            Capture {
                id: 1,
                text: "first".into(),
                complete: true,
            },
            Capture {
                id: 2,
                text: "second".into(),
                complete: false,
            },
        ];
        write_snapshot(&path, &first).unwrap();
        let before = std::fs::read(&path).unwrap();
        write_snapshot(&path, &second).unwrap();
        let after = std::fs::read(&path).unwrap();
        assert_ne!(before, after);
        assert_eq!(after, serde_json::to_vec_pretty(&second).unwrap());
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn event_channel_delivers_without_polling() {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
        tx.unbounded_send("ready".to_owned()).unwrap();
        tx.unbounded_send("text".to_owned()).unwrap();
        drop(tx);
        let collected = futures::executor::block_on(async {
            let mut out = Vec::new();
            while let Some(msg) = futures::StreamExt::next(&mut rx).await {
                out.push(msg);
            }
            out
        });
        assert_eq!(collected, vec!["ready".to_owned(), "text".to_owned()]);
    }

    #[test]
    fn persist_queue_is_fifo_ordered() {
        let dir = std::env::temp_dir().join(format!(
            "cooper-perf-fifo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("captures.json");
        let (completion, mut results) = futures::channel::mpsc::unbounded();
        let mut worker = PersistWorker::new(completion);
        let first = sample_items();
        let second = vec![Capture {
            id: 3,
            text: "third snapshot".into(),
            complete: false,
        }];
        for (seq, snapshot) in [(1, first), (2, second.clone())] {
            assert!(worker.submit(PersistRequest {
                seq,
                snapshot,
                path: path.clone(),
            }));
        }
        worker.shutdown();
        let completed = futures::executor::block_on(async {
            let mut seqs = Vec::new();
            while let Some(result) = futures::StreamExt::next(&mut results).await {
                result.result.unwrap();
                seqs.push(result.seq);
            }
            seqs
        });
        assert_eq!(completed, vec![1, 2]);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, serde_json::to_vec_pretty(&second).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deferred_capture_waits_for_save_and_non_edit_state() {
        assert!(!can_deliver_deferred_capture(true, false, false));
        assert!(!can_deliver_deferred_capture(false, true, false));
        assert!(!can_deliver_deferred_capture(false, false, true));
        assert!(can_deliver_deferred_capture(false, false, false));
    }
}

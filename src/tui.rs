use crate::{
    api::{Api, File},
    download::{self, Stats},
    store::Store,
};
use anyhow::{Result, ensure};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    collections::VecDeque,
    io::IsTerminal,
    net::IpAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Options {
    pub folder: String,
    pub output: PathBuf,
    pub ip: Option<IpAddr>,
    pub sni: Option<String>,
    pub connections: u8,
    pub ca_cert: Option<PathBuf>,
}
enum Message {
    Folder(String, Result<Vec<File>, String>),
    Preview(String, Result<Vec<File>, String>),
    Finished(Result<(), String>),
}
struct Active {
    id: String,
    name: String,
    stats: Stats,
    rx: Receiver<Stats>,
    cancel: Arc<AtomicBool>,
    worker: thread::JoinHandle<()>,
}
struct View {
    path: Vec<(String, String)>,
    files: Vec<File>,
    selection: ListState,
    preview: Vec<File>,
    preview_id: Option<String>,
    status: String,
    loading: bool,
    connections: u8,
    active: Option<Active>,
    queue: VecDeque<File>,
    quitting: bool,
}
impl View {
    fn selected(&self) -> Option<&File> {
        self.selection.selected().and_then(|i| self.files.get(i))
    }
    fn move_by(&mut self, delta: isize) {
        if self.files.is_empty() {
            self.selection.select(None);
            return;
        }
        let next = (self.selection.selected().unwrap_or(0) as isize + delta)
            .clamp(0, self.files.len() as isize - 1) as usize;
        self.selection.select(Some(next));
        self.preview.clear();
        self.preview_id = None;
    }
    fn draw(&mut self, frame: &mut Frame) {
        let [header, browser, progress, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(6),
            Constraint::Length(2),
        ])
        .areas(frame.area());
        let path = self
            .path
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                " gdirect ".black().on_cyan().bold(),
                format!("  {path}").cyan(),
                format!("   {} connections / download", self.connections).gray(),
            ])),
            header,
        );
        let [list, preview] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(browser);
        let items: Vec<_> = self
            .files
            .iter()
            .map(|f| {
                ListItem::new(Line::from(vec![
                    if f.folder() {
                        " ▸ ".cyan()
                    } else {
                        "   ".white()
                    },
                    f.name.clone().fg(if f.folder() {
                        Color::Cyan
                    } else {
                        Color::White
                    }),
                ]))
            })
            .collect();
        let title = if self.loading {
            " Files · refreshing ".into()
        } else {
            format!(" Files · {} ", self.files.len())
        };
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(title))
                .highlight_style(Style::default().bg(Color::DarkGray).bold())
                .highlight_symbol("› "),
            list,
            &mut self.selection,
        );
        let lines = match self.selected() {
            Some(file) if file.folder() => {
                let mut lines = vec![
                    Line::from(file.name.clone()).cyan().bold(),
                    Line::from("Folder"),
                    Line::from(""),
                ];
                if self.preview_id.as_deref() == Some(&file.id) && self.preview.is_empty() {
                    lines.push(Line::from("Empty folder / loading…").dark_gray());
                }
                lines.extend(self.preview.iter().map(|f| {
                    Line::from(format!(
                        "{}{}",
                        if f.folder() { "▸ " } else { "  " },
                        f.name
                    ))
                }));
                lines
            }
            Some(file) => vec![
                Line::from(file.name.clone()).bold(),
                Line::from(""),
                Line::from(file.mime_type.clone()),
                Line::from(format!(
                    "Size     {}",
                    file.size
                        .as_ref()
                        .and_then(|s| s.parse().ok())
                        .map(bytes)
                        .unwrap_or_else(|| "—".into())
                )),
                Line::from(format!(
                    "Modified {}",
                    file.modified_time.as_deref().unwrap_or("—")
                )),
                Line::from(""),
                Line::from(if file.workspace() {
                    "d  Export document as PDF"
                } else {
                    "d  Add to download queue"
                })
                .cyan(),
            ],
            None => vec![Line::from(if self.loading {
                "Loading Drive…"
            } else {
                "This folder is empty"
            })],
        };
        frame.render_widget(
            Paragraph::new(lines)
                .block(Block::bordered().title(" Preview "))
                .wrap(Wrap { trim: false }),
            preview,
        );
        let block = Block::bordered().title(format!(" Downloads · {} queued ", self.queue.len()));
        let inner = block.inner(progress);
        frame.render_widget(block, progress);
        let [name, gauge, details, message] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(inner);
        if let Some(active) = &self.active {
            let s = active.stats;
            let ratio = if s.total > 0 {
                (s.done as f64 / s.total as f64).clamp(0., 1.)
            } else {
                0.
            };
            frame.render_widget(Paragraph::new(active.name.clone()).bold(), name);
            frame.render_widget(
                Gauge::default()
                    .gauge_style(Style::default().fg(Color::Cyan))
                    .ratio(ratio)
                    .label(if s.total > 0 {
                        format!("{:.1}%", ratio * 100.)
                    } else {
                        "Connecting…".into()
                    }),
                gauge,
            );
            let eta = s
                .total
                .saturating_sub(s.done)
                .checked_div(s.speed)
                .map(|seconds| format!("{seconds}s"))
                .unwrap_or_else(|| "—".into());
            frame.render_widget(
                Paragraph::new(format!(
                    "{} / {}   {}/s   {} active connections   ETA {eta}",
                    bytes(s.done),
                    bytes(s.total),
                    bytes(s.speed),
                    s.connections
                )),
                details,
            );
        } else {
            frame.render_widget(
                Paragraph::new("Ready · select a file and press d").dark_gray(),
                name,
            );
        }
        frame.render_widget(Paragraph::new(self.status.clone()).yellow(), message);
        frame.render_widget(Paragraph::new(" j/k ↑↓ move  h/← back  l/Enter open  d download  +/- next job connections\n r refresh  c cancel active  q quit (saves partial download)"),footer);
    }
}
fn bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}
fn fetch(api: &Api, tx: &Sender<Message>, id: String, preview: bool) {
    let api = api.clone();
    let tx = tx.clone();
    thread::spawn(move || {
        let result = api.list(&id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(if preview {
            Message::Preview(id, result)
        } else {
            Message::Folder(id, result)
        });
    });
}
fn start(
    api: &Api,
    options: &Options,
    tx: &Sender<Message>,
    file: File,
    connections: u8,
) -> Active {
    let (progress_tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let api = api.clone();
    let tx = tx.clone();
    let id = file.id.clone();
    let name = file.name.clone();
    let output = crate::api::destination(&options.output, &file);
    let mut args = download::Args {
        file: file.id.clone(),
        output,
        ip: options.ip,
        sni: options.sni.clone(),
        token: None,
        export: None,
        connections,
        ca_cert: options.ca_cert.clone(),
    };
    if file.workspace() {
        args.export = Some("application/pdf".into());
        args.output
            .set_file_name(format!("{}.pdf", crate::api::safe_name(&file.name)));
    }
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            args.token = Some(api.download_token()?);
            if worker_cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Cancelled");
            }
            let result = download::run(
                args,
                Some(&mut download::Progress {
                    tx: progress_tx,
                    cancel: worker_cancel.clone(),
                }),
            );
            if worker_cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Cancelled; partial download saved");
            }
            result
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Message::Finished(result));
    });
    Active {
        id,
        name,
        stats: Stats::default(),
        rx,
        cancel,
        worker,
    }
}
pub fn run(api: Api, options: Options) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "browse needs a terminal; use gdirect ls --json for scripts"
    );
    api.token()?;
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, api, options);
    ratatui::restore();
    result
}
fn event_loop(terminal: &mut ratatui::DefaultTerminal, api: Api, options: Options) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let db = Store::open(&api.db)?;
    let initial = api.cached(&options.folder)?;
    let folder_name = if options.folder == "root" {
        "My Drive".to_owned()
    } else {
        api.metadata(&options.folder)?.name
    };
    let mut view = View {
        path: vec![(options.folder.clone(), folder_name)],
        files: initial,
        selection: ListState::default().with_selected(Some(0)),
        preview: vec![],
        preview_id: None,
        status: String::new(),
        loading: true,
        connections: options.connections,
        active: None,
        queue: VecDeque::new(),
        quitting: false,
    };
    fetch(&api, &tx, options.folder.clone(), false);
    let mut changed = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Folder(id, result)
                        if view.path.last().is_some_and(|(current, _)| current == &id) =>
                    {
                        view.loading = false;
                        match result {
                            Ok(files) => {
                                view.files = files;
                                view.selection.select(if view.files.is_empty() {
                                    None
                                } else {
                                    Some(0)
                                });
                                view.preview_id = None;
                                view.status.clear();
                                changed = Instant::now();
                            }
                            Err(e) => view.status = e,
                        }
                    }
                    Message::Preview(id, result) if view.selected().is_some_and(|f| f.id == id) => {
                        match result {
                            Ok(files) => view.preview = files,
                            Err(e) => view.status = e,
                        }
                    }
                    Message::Finished(result) => {
                        if let Some(active) = view.active.take() {
                            let _ = active.worker.join();
                            view.status = match result {
                                Ok(()) => format!("Saved {}", active.name),
                                Err(e) => e,
                            };
                        }
                    }
                    _ => {}
                }
            }
            if let Some(active) = &mut view.active {
                while let Ok(stats) = active.rx.try_recv() {
                    active.stats = stats;
                }
            }
            if view.active.is_none() {
                if view.quitting {
                    break;
                }
                if let Some(file) = view.queue.pop_front() {
                    view.active = Some(start(&api, &options, &tx, file, view.connections));
                }
            }
            if changed.elapsed() > Duration::from_millis(200)
                && !view.loading
                && let Some(file) = view.selected().filter(|f| f.folder())
            {
                let id = file.id.clone();
                if view.preview_id.as_deref() != Some(&id) {
                    view.preview = api.cached(&id).unwrap_or_default();
                    view.preview_id = Some(id.clone());
                    fetch(&api, &tx, id, true);
                }
            }
            terminal.draw(|frame| view.draw(frame))?;
            if !event::poll(Duration::from_millis(80))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if view.quitting {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('c')
                    if key.code == KeyCode::Char('q')
                        || key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    view.quitting = true;
                    view.queue.clear();
                    view.status = "Stopping download…".into();
                    if let Some(active) = &view.active {
                        active.cancel.store(true, Ordering::Relaxed);
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(active) = &view.active {
                        active.cancel.store(true, Ordering::Relaxed);
                        view.status = "Cancelling…".into();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    view.move_by(1);
                    changed = Instant::now();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    view.move_by(-1);
                    changed = Instant::now();
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    view.connections = (view.connections + 1).min(16);
                    db.set("connections", &view.connections.to_string())?;
                }
                KeyCode::Char('-') => {
                    view.connections = view.connections.saturating_sub(1).max(1);
                    db.set("connections", &view.connections.to_string())?;
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(file) = view.selected().filter(|f| f.folder()).cloned() {
                        view.path.push((file.id.clone(), file.name));
                        view.files = api.cached(&file.id).unwrap_or_default();
                        view.selection.select(Some(0));
                        view.preview.clear();
                        view.preview_id = None;
                        view.loading = true;
                        fetch(&api, &tx, file.id, false);
                    }
                }
                KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                    if view.path.len() > 1 {
                        view.path.pop();
                        let id = view.path.last().unwrap().0.clone();
                        view.files = api.cached(&id).unwrap_or_default();
                        view.selection.select(Some(0));
                        view.preview.clear();
                        view.preview_id = None;
                        view.loading = true;
                        fetch(&api, &tx, id, false);
                    }
                }
                KeyCode::Char('r') => {
                    view.loading = true;
                    fetch(&api, &tx, view.path.last().unwrap().0.clone(), false);
                }
                KeyCode::Char('d') => {
                    if let Some(file) = view.selected().filter(|f| !f.folder()).cloned()
                        && !view.queue.iter().any(|f| f.id == file.id)
                        && !view.active.as_ref().is_some_and(|a| a.id == file.id)
                    {
                        view.status = format!("Queued {}", file.name);
                        view.queue.push_back(file);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();
    // Always stop/join the native worker before restoring the terminal, even on I/O failure.
    if let Some(active) = view.active.take() {
        active.cancel.store(true, Ordering::Relaxed);
        let _ = active.worker.join();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_has_browser_above_downloads() {
        let mut view = View {
            path: vec![("root".into(), "My Drive".into())],
            files: vec![File {
                id: "folder".into(),
                name: "Documents".into(),
                mime_type: "application/vnd.google-apps.folder".into(),
                size: None,
                modified_time: None,
                md5_checksum: None,
                resource_key: None,
            }],
            selection: ListState::default().with_selected(Some(0)),
            preview: vec![],
            preview_id: None,
            status: "Ready".into(),
            loading: false,
            connections: 8,
            active: None,
            queue: VecDeque::new(),
            quitting: false,
        };
        for (w, h) in [(100, 30), (40, 15)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| view.draw(f)).unwrap();
            let buffer = terminal.backend().buffer();
            let rows: Vec<String> = (0..h)
                .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let files = rows.iter().position(|s| s.contains("Files")).unwrap();
            let downloads = rows.iter().position(|s| s.contains("Downloads")).unwrap();
            assert!(downloads > files);
            assert!(rows.iter().any(|s| s.contains("Documents")));
        }
    }
}

use crate::{
    client::Switch,
    config::Config,
    ports::{self, Port},
};
use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    collections::BTreeSet,
    io::{self, IsTerminal},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};
use unicode_width::UnicodeWidthChar;

enum Job {
    Refresh,
    Cycle(u64),
}
struct Update {
    ports: Option<Vec<Port>>,
    message: String,
    failed: bool,
}

fn perform(path: &std::path::Path, job: Job) -> Result<Update> {
    let config = Config::load(path)?;
    let switch = Switch::login(&config)?;
    let data = ports::snapshot(&switch, &config)?;
    match job {
        Job::Refresh => Ok(Update {
            ports: Some(data),
            message:
                "Fresh snapshot · blue: PoE · green: link only · gray: offline · amber: unknown"
                    .into(),
            failed: false,
        }),
        Job::Cycle(port) => {
            ports::validate_cycle(&data, &[port])?;
            switch.reset(&[port])?;
            // Only read back after acceptance. Never retry the reset if readback fails.
            match ports::snapshot(&switch, &config) {
                Ok(data) => Ok(Update {
                    ports: Some(data),
                    message: format!("Port {port}: reset accepted · device recovery unverified"),
                    failed: false,
                }),
                Err(error) => Ok(Update {
                    ports: None,
                    message: format!(
                        "Port {port}: reset accepted, readback failed: {error:#}. Refresh; do not blindly retry."
                    ),
                    failed: true,
                }),
            }
        }
    }
}

fn worker(path: PathBuf) -> (Sender<Job>, Receiver<Update>) {
    let (jobs, incoming) = mpsc::channel();
    let (outgoing, updates) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(job) = incoming.recv() {
            let update = perform(&path, job).unwrap_or_else(|e| Update {
                ports: None,
                message: format!("{e:#}"),
                failed: true,
            });
            if outgoing.send(update).is_err() {
                break;
            }
        }
    });
    (jobs, updates)
}

#[derive(Default)]
struct App {
    ports: Vec<Port>,
    selected: usize,
    expanded: BTreeSet<(u64, u64, u64)>,
    scroll: usize,
    detail_scroll: usize,
    busy: bool,
    quit_pending: bool,
    failed: bool,
    message: String,
}

fn key(port: &Port) -> (u64, u64, u64) {
    (port.unit, port.slot, port.port)
}
fn clean(text: &str) -> String {
    // Device-provided names must not inject terminal control characters.
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

impl App {
    fn update(&mut self, update: Update) {
        let selected = self.ports.get(self.selected).map(key);
        if let Some(data) = update.ports {
            self.ports = data;
            self.selected = selected
                .and_then(|k| self.ports.iter().position(|p| key(p) == k))
                .unwrap_or(0);
        } else {
            // Do not render old delivery state as if it were a fresh measurement.
            self.ports.clear();
            self.selected = 0;
            self.scroll = 0;
        }
        self.message = update.message;
        self.failed = update.failed;
        self.busy = false;
    }

    fn send(&mut self, sender: &Sender<Job>, job: Job) -> Result<()> {
        if self.busy {
            return Ok(());
        }
        self.message = match job {
            Job::Refresh => "Reading switch...".into(),
            Job::Cycle(p) => format!("Power-cycling port {p}..."),
        };
        sender.send(job).context("TUI network worker stopped")?;
        self.busy = true;
        self.failed = false;
        Ok(())
    }

    fn cycle(&mut self, sender: &Sender<Job>) -> Result<()> {
        if self.busy {
            return Ok(());
        }
        if let Some(port) = self.ports.get(self.selected) {
            if let Err(e) = ports::validate_cycle(&self.ports, &[port.port]) {
                self.message = e.to_string();
                self.failed = true;
            } else {
                self.send(sender, Job::Cycle(port.port))?;
            }
        }
        Ok(())
    }

    fn toggle_evidence(&mut self) {
        self.detail_scroll = 0;
        if let Some(port) = self.ports.get(self.selected) {
            let k = key(port);
            if !self.expanded.remove(&k) {
                self.expanded.insert(k);
            }
        }
    }

    fn rows(&self) -> Vec<(usize, Vec<String>)> {
        self.ports
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let label = match p.attachment.status {
                    "lldp_neighbor" => "LLDP neighbor",
                    "switch_ui_neighbor" => "Switch UI neighbor",
                    "single_mac_candidate" => "MAC candidate (unverified)",
                    _ => "Physical device unknown",
                };
                let mut lines = vec![label.to_owned()];
                if let Some(d) = &p.attachment.device {
                    let name = d
                        .name
                        .as_deref()
                        .or(d.vendor.as_deref())
                        .unwrap_or("Unknown device");
                    let ips = d.ips.iter().cloned().collect::<Vec<_>>().join(", ");
                    lines.push(format!(
                        "{} | {}{}",
                        clean(name),
                        d.mac,
                        if ips.is_empty() {
                            String::new()
                        } else {
                            format!(" | {}", clean(&ips))
                        }
                    ));
                } else {
                    lines.push(clean(p.attachment.explanation));
                }
                let expanded = self.expanded.contains(&key(p));
                lines.push(format!(
                    "{} evidence ({} MACs)",
                    if expanded { "▾ Hide" } else { "▸ Show" },
                    p.attachment.learned_mac_count
                ));
                if expanded {
                    lines.push(clean(p.attachment.explanation));
                    if let Some(desc) = p
                        .attachment
                        .device
                        .as_ref()
                        .and_then(|d| d.description.as_ref())
                    {
                        lines.push(clean(desc));
                    }
                    lines.push(
                        "Learned through this port; not necessarily directly attached:".into(),
                    );
                    for d in &p.devices {
                        lines.push(clean(&format!(
                            "{}: {} {} {}",
                            d["source"].as_str().unwrap_or("?"),
                            d["hostName"].as_str().unwrap_or("unknown"),
                            d["hostIpAddress"].as_str().unwrap_or(""),
                            d["hostMacAddress"].as_str().unwrap_or("")
                        )));
                    }
                }
                (i, lines)
            })
            .collect()
    }
}

fn wrap_lines(lines: &[String], width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut output = Vec::new();
    for line in lines {
        let mut text = String::new();
        let mut columns = 0;
        for ch in line.chars() {
            let size = ch.width().unwrap_or(0);
            if columns + size > width && !text.is_empty() {
                output.push(std::mem::take(&mut text));
                columns = 0;
            }
            text.push(ch);
            columns += size;
        }
        output.push(text);
    }
    output
}

fn port_color(port: &Port) -> Color {
    if port.poe_powered == Some(true) {
        return Color::Rgb(0, 136, 255);
    }
    if (port.poe_capable && port.poe_powered.is_none()) || port.link_up.is_none() {
        return Color::Rgb(255, 184, 64);
    }
    if port.link_up == Some(true) {
        Color::Rgb(0, 221, 85)
    } else {
        Color::DarkGray
    }
}

fn port_title(port: &Port) -> String {
    let name = port
        .attachment
        .device
        .as_ref()
        .and_then(|d| d.name.as_deref().or(d.vendor.as_deref()))
        .map(clean)
        .filter(|n| !n.trim().is_empty());
    match name {
        Some(name) => format!("Port {} · {name}", port.port),
        None => format!("Port {}", port.port),
    }
}

fn bolt_style(port: &Port) -> Style {
    if !port.poe_capable {
        Style::default().fg(Color::DarkGray)
    } else {
        match port.poe_powered {
            Some(true) => Style::default()
                .fg(port_color(port))
                .add_modifier(Modifier::REVERSED),
            Some(false) => Style::default().fg(port_color(port)),
            None => Style::default().fg(port_color(port)),
        }
    }
}

// Returns hit regions for mouse selection and bolt activation.
fn draw(frame: &mut ratatui::Frame, app: &mut App) -> Vec<(Rect, Rect, usize)> {
    let area = frame.area();
    if area.width < 35 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Resize terminal to at least 35×10. q: quit"),
            area,
        );
        return Vec::new();
    }
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new("NETGEAR PoE").style(Style::default().add_modifier(Modifier::BOLD)),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(clean(&app.message))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(if app.failed { Color::Red } else { Color::Gray })),
        sections[1],
    );
    frame.render_widget(Paragraph::new("↑↓/jk select · e evidence · PgUp/Dn scroll · r refresh · q quit\nSpace/⚡ click: IMMEDIATE power cycle (no confirmation)").style(Style::default().fg(Color::Gray)), sections[3]);
    let content = sections[2];
    let mut rows = app.rows();
    for (_, lines) in &mut rows {
        *lines = wrap_lines(lines, content.width.saturating_sub(2) as usize);
    }
    let selected = app.selected;
    if let Some((_, lines)) = rows.get_mut(selected) {
        app.detail_scroll = app.detail_scroll.min(lines.len().saturating_sub(1));
        lines.drain(..app.detail_scroll);
    }
    app.scroll = app.scroll.min(selected);
    let height = |i: usize| rows[i].1.len() as u16 + 2;
    while app.scroll < selected
        && (app.scroll..=selected)
            .map(|i| height(i) as usize)
            .sum::<usize>()
            > content.height as usize
    {
        app.scroll += 1;
    }
    let mut y = content.y;
    let mut hits = Vec::new();
    for (i, lines) in rows.iter().skip(app.scroll) {
        if y >= content.bottom() {
            break;
        }
        let p = &app.ports[*i];
        let h = height(*i).min(content.bottom() - y);
        let row = Rect::new(content.x, y, content.width, h);
        let selected = *i == app.selected;
        let link = match p.link_up {
            Some(true) => "up",
            Some(false) => "down",
            None => "?",
        };
        let power = p
            .power_watts
            .map(|w| format!("{w:.1} W"))
            .unwrap_or_else(|| "no PoE".into());
        let summary = format!(
            "{} {} | {} | {} | {}{}",
            if selected { "›" } else { " " },
            port_title(p),
            link,
            clean(&p.speed),
            power,
            if p.protected { " | protected" } else { "" }
        );
        let header = Rect::new(row.x, row.y, row.width.saturating_sub(5), 1);
        frame.render_widget(
            Paragraph::new(summary).style(Style::default().fg(port_color(p)).add_modifier(
                if selected {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                },
            )),
            header,
        );
        let bolt = Rect::new(row.right() - 5, row.y, 5, 1);
        frame.render_widget(
            Paragraph::new(if app.busy {
                " ... "
            } else if p.poe_capable && p.poe_powered.is_none() {
                " ⚡? "
            } else {
                " ⚡  "
            })
            .style(bolt_style(p)),
            bolt,
        );
        if h > 1 {
            let details = Rect::new(row.x + 2, row.y + 1, row.width - 2, h.saturating_sub(2));
            frame.render_widget(
                Paragraph::new(
                    lines
                        .iter()
                        .map(|s| Line::from(Span::raw(s.clone())))
                        .collect::<Vec<_>>(),
                )
                .style(Style::default().fg(Color::Gray)),
                details,
            );
        }
        if h >= 2 {
            frame.render_widget(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
                row,
            );
        }
        hits.push((row, bolt, *i));
        y += h;
    }
    hits
}

struct Screen;
fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}
impl Drop for Screen {
    fn drop(&mut self) {
        restore();
    }
}

pub fn run(path: PathBuf) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "tui requires a terminal; use ssh -t user@host 'netgear-poe tui'"
    );
    let _ = Config::load(&path)?; // Fail before changing terminal mode.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    let _screen = Screen;
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, receiver) = worker(path);
    let mut app = App::default();
    app.send(&sender, Job::Refresh)?;
    loop {
        while let Ok(update) = receiver.try_recv() {
            app.update(update);
        }
        if app.quit_pending && !app.busy {
            break;
        }
        let mut hits = Vec::new();
        terminal.draw(|frame| hits = draw(frame, &mut app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if !app.busy {
                        break;
                    }
                    app.quit_pending = true;
                    app.message = "Finishing current request before exiting...".into();
                }
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    if !app.busy {
                        break;
                    }
                    app.quit_pending = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.detail_scroll = 0;
                    app.selected = (app.selected + 1).min(app.ports.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.detail_scroll = 0;
                    app.selected = app.selected.saturating_sub(1);
                }
                KeyCode::Home => {
                    app.detail_scroll = 0;
                    app.selected = 0;
                }
                KeyCode::End => {
                    app.detail_scroll = 0;
                    app.selected = app.ports.len().saturating_sub(1);
                }
                KeyCode::PageDown => app.detail_scroll = app.detail_scroll.saturating_add(5),
                KeyCode::PageUp => app.detail_scroll = app.detail_scroll.saturating_sub(5),
                KeyCode::Enter | KeyCode::Char('e') => app.toggle_evidence(),
                KeyCode::Char('r') if !app.quit_pending => app.send(&sender, Job::Refresh)?,
                KeyCode::Char(' ') if !app.quit_pending => app.cycle(&sender)?,
                _ => {}
            },
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some((_, bolt, i)) = hits
                        .iter()
                        .find(|(r, _, _)| r.contains((m.column, m.row).into()))
                    {
                        app.selected = *i;
                        app.detail_scroll = 0;
                        if bolt.contains((m.column, m.row).into()) && !app.quit_pending {
                            app.cycle(&sender)?;
                        }
                    }
                }
                MouseEventKind::ScrollDown => {
                    app.detail_scroll = 0;
                    app.selected = (app.selected + 1).min(app.ports.len().saturating_sub(1))
                }
                MouseEventKind::ScrollUp => {
                    app.detail_scroll = 0;
                    app.selected = app.selected.saturating_sub(1);
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn data() -> Vec<Port> {
        ports::merge(&json!({"switchPortConfig":{"unit":[{"id":1,"slot":[{"id":1,"port":[{"portNum":1,"linkState":0},{"portNum":9,"linkState":0}]}]}]}}),&json!({"poePortConfig":[{"unit":1,"portNum":1,"enable":true,"status":2,"currentPower":5200}]}),&[],&[]).unwrap()
    }
    #[test]
    fn busy_blocks_duplicate_jobs_and_protected_ports_do_not_enqueue() {
        let (tx, rx) = mpsc::channel();
        let mut app = App {
            ports: data(),
            ..Default::default()
        };
        app.ports[0].protected = true;
        app.cycle(&tx).unwrap();
        assert!(rx.try_recv().is_err());
        app.ports[0].protected = false;
        app.cycle(&tx).unwrap();
        app.cycle(&tx).unwrap();
        assert!(matches!(rx.try_recv().unwrap(), Job::Cycle(1)));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn render_contains_separators_and_inverse_power_with_no_confirmation() {
        let mut app = App {
            ports: data(),
            ..Default::default()
        };
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, &mut app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Port 1"));
        assert!(text.contains("Port 9"));
        assert!(text.contains("─"));
        assert!(text.contains("IMMEDIATE"));
        assert!(
            buffer
                .content
                .iter()
                .any(|c| c.modifier.contains(Modifier::REVERSED))
        );
        app.toggle_evidence();
        assert!(
            app.rows()[0]
                .1
                .iter()
                .any(|s| s.contains("Learned through"))
        );
        assert_eq!(clean("evil\x1b[31m\nname"), "evil [31m name");
    }
    #[test]
    fn narrow_terminal_wraps_evidence_and_failed_updates_clear_stale_power() {
        assert_eq!(wrap_lines(&["abcdef".into()], 3), vec!["abc", "def"]);
        assert_eq!(wrap_lines(&["界界".into()], 2), vec!["界", "界"]);
        let mut app = App {
            ports: data(),
            ..Default::default()
        };
        app.update(Update {
            ports: None,
            message: "offline".into(),
            failed: true,
        });
        assert!(app.ports.is_empty());
        assert!(app.failed);
        assert!(!app.busy);
    }
    #[test]
    fn titles_use_name_then_mac_vendor_and_state_colors_match_netgear() {
        let mut port = data().remove(0);
        port.attachment = crate::identity::identify(
            &[json!({"hostMacAddress":"D4:6C:62:00:00:01","source":"arp"})],
            Some(true),
            &Default::default(),
            &crate::identity::Vendors::default(),
        );
        port.attachment.device.as_mut().unwrap().vendor = Some("MultiTracks.com, LLC".into());
        assert_eq!(port_title(&port), "Port 1 · MultiTracks.com, LLC");
        port.attachment.device.as_mut().unwrap().name = Some("Lab device".into());
        assert_eq!(port_title(&port), "Port 1 · Lab device");
        port.link_up = Some(false);
        assert_eq!(port_color(&port), Color::Rgb(0, 136, 255)); // PoE beats link-down.
        port.poe_powered = Some(false);
        port.link_up = Some(true);
        assert_eq!(port_color(&port), Color::Rgb(0, 221, 85));
        port.link_up = Some(false);
        assert_eq!(port_color(&port), Color::DarkGray);
        port.poe_powered = None;
        assert_eq!(port_color(&port), Color::Rgb(255, 184, 64));
        port.attachment.device = None;
        assert_eq!(port_title(&port), "Port 1");
    }
}

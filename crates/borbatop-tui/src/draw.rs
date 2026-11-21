//! Rendering: turns the [`App`] state into ratatui widgets.

use borbatop_core::{ActiveState, Permille, Pressure, UnitRow};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{
    App, InputMode, LogView, MessageKind, PacmanState, ProcessLayout, SignalDialog, Visibility,
    VisibleRow,
};
use crate::input::{SignalChoice, SortKey, Tab};
use crate::model::{Model, PacmanView, ProcessRow};
use crate::theme::Theme;
use crate::widgets::{Heat, bar_text, fit, fit_right, heat_of, safe, scale_of, sparkline};

/// The logo: the wordmark followed by an Electric Blue dot.
pub const WORDMARK: &str = "BORBA";
const WORDMARK_DOT: &str = ".";
const PRODUCT_NAME: &str = "borbatop";
const MAX_CORE_ROWS: usize = 8;
const MIN_TOP_HEIGHT: u16 = 6;
const BORDER_ROWS: u16 = 2;
const CPU_PANEL_PERCENT: u16 = 55;
const DIALOG_WIDTH: u16 = 46;
const DIALOG_HEIGHT: u16 = 9;
const NARROW_WIDTH: usize = 100;
const WIDE_WIDTH: usize = 120;
const NOT_AVAILABLE: &str = "n/a";
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_DAY: u64 = 86_400;
const SPARK_RESERVED: usize = 16;
const NET_SPARK_WIDTH: usize = 30;
const PID_WIDTH: usize = 7;
const USER_WIDTH: usize = 10;
const STATE_WIDTH: usize = 2;
const CPU_WIDTH: usize = 6;
const MEMORY_WIDTH: usize = 7;
const THREADS_WIDTH: usize = 4;
const UNIT_WIDTH: usize = 26;
const PACKAGE_WIDTH: usize = 16;
const SERVICE_NAME_WIDTH: usize = 40;
const ACTIVE_WIDTH: usize = 12;
const SUB_WIDTH: usize = 10;
const MOUNT_WIDTH: usize = 16;
const FS_TYPE_WIDTH: usize = 6;
const INTERFACE_WIDTH: usize = 12;
const RATE_WIDTH: usize = 10;
const DIALOG_RESERVED: usize = 12;
const HEADER_INFO_WIDTH: u16 = 48;
const JOURNAL_MARGIN_X: u16 = 4;
const JOURNAL_MARGIN_Y: u16 = 2;

struct Styles {
    base: Style,
    secondary: Style,
    accent: Style,
    border: Style,
    danger: Style,
    ok: Style,
    selected: Style,
    strong: Style,
}

impl Styles {
    fn of(theme: Theme) -> Self {
        let base = Style::new().fg(theme.foreground).bg(theme.background);
        Self {
            base,
            secondary: base.fg(theme.secondary),
            accent: base.fg(theme.accent),
            border: base.fg(theme.border),
            danger: base.fg(theme.danger),
            ok: base.fg(theme.ok),
            selected: Style::new()
                .fg(theme.selected_foreground)
                .bg(theme.selected_background),
            strong: base.add_modifier(Modifier::BOLD),
        }
    }

    fn heat(&self, heat: Heat) -> Style {
        match heat {
            Heat::Normal => self.base,
            Heat::Busy => self.accent,
            Heat::Hot => self.danger,
        }
    }

    fn bar(&self, heat: Heat) -> Style {
        match heat {
            Heat::Hot => self.danger,
            Heat::Normal | Heat::Busy => self.accent,
        }
    }
}

/// Draws the whole screen.
///
/// # Arguments
///
/// * `frame` - frame to draw into
/// * `app` - state to show
pub fn draw(frame: &mut Frame, app: &App) {
    let styles = Styles::of(Theme::of(app.config.theme));
    frame.render_widget(Block::new().style(styles.base), frame.area());

    let top_height = top_height(&app.model);
    let [header, top, tabs, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(top_height),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, header, app, &styles);
    draw_top(frame, top, &app.model, &styles);
    draw_tabs(frame, tabs, app, &styles);
    match app.tab {
        Tab::Processes => draw_processes(frame, body, app, &styles),
        Tab::Services => draw_services(frame, body, app, &styles),
        Tab::Pacman => draw_pacman(frame, body, app, &styles),
        Tab::Io => draw_io(frame, body, app, &styles),
    }
    draw_status(frame, status, app, &styles);

    if let Some(dialog) = &app.dialog {
        draw_dialog(frame, dialog, &styles);
    }
    if let Some(logs) = &app.logs {
        draw_logs(frame, logs, &styles);
    }
}

fn top_height(model: &Model) -> u16 {
    let rows = model.cores.len().div_ceil(2).clamp(2, MAX_CORE_ROWS);
    let rows = u16::try_from(rows).unwrap_or(2);
    rows.saturating_add(BORDER_ROWS)
        .saturating_add(1)
        .max(MIN_TOP_HEIGHT)
}

fn to_width(area: Rect) -> usize {
    usize::from(area.width)
}

fn panel<'a>(title: &str, styles: &Styles) -> Block<'a> {
    Block::bordered()
        .border_style(styles.border)
        .style(styles.base)
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            styles.accent,
        ))
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(HEADER_INFO_WIDTH)])
            .areas(area);
    let logo = Line::from(vec![
        Span::styled(WORDMARK, styles.strong),
        Span::styled(WORDMARK_DOT, styles.accent.add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  {PRODUCT_NAME} {}", env!("CARGO_PKG_VERSION")),
            styles.accent,
        ),
    ]);
    frame.render_widget(Paragraph::new(logo).style(styles.base), left);

    let kernel = if app.model.kernel_release.is_empty() {
        String::from("linux")
    } else {
        format!("linux {}", app.model.kernel_release)
    };
    let state = if app.is_paused() {
        Span::styled("  ⏸ PAUSED", styles.danger)
    } else {
        Span::styled("  ● live", styles.ok)
    };
    let info = Line::from(vec![Span::styled(kernel, styles.secondary), state]);
    frame.render_widget(
        Paragraph::new(info)
            .alignment(Alignment::Right)
            .style(styles.base),
        right,
    );
}

fn draw_top(frame: &mut Frame, area: Rect, model: &Model, styles: &Styles) {
    let [cpu, memory] = Layout::horizontal([
        Constraint::Percentage(CPU_PANEL_PERCENT),
        Constraint::Percentage(100_u16.saturating_sub(CPU_PANEL_PERCENT)),
    ])
    .areas(area);
    draw_cpu(frame, cpu, model, styles);
    draw_memory(frame, memory, model, styles);
}

fn draw_cpu(frame: &mut Frame, area: Rect, model: &Model, styles: &Styles) {
    let block = panel(&format!("CPU · {} cores", model.cores.len()), styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = model.cores.len().div_ceil(2).min(MAX_CORE_ROWS);
    let half = to_width(inner).saturating_div(2);
    let bar_width = half.saturating_sub(11).clamp(4, 40);
    let mut lines: Vec<Line> = Vec::with_capacity(rows.saturating_add(1));

    for row in 0..rows {
        let mut spans = core_cell(model, row, bar_width, styles);
        spans.push(Span::raw("  "));
        spans.extend(core_cell(
            model,
            row.saturating_add(rows),
            bar_width,
            styles,
        ));
        lines.push(Line::from(spans));
    }

    let total = model.cpu_history.latest().copied().unwrap_or(0);
    let spark_width = to_width(inner).saturating_sub(SPARK_RESERVED);
    lines.push(Line::from(vec![
        Span::styled("TOTAL ", styles.secondary),
        Span::styled(
            sparkline(model.cpu_history.iter(), 1_000, spark_width),
            styles.accent,
        ),
        Span::styled(format!(" {}%", Permille::new(total)), styles.base),
    ]));
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

fn core_cell<'a>(model: &Model, index: usize, bar_width: usize, styles: &Styles) -> Vec<Span<'a>> {
    let Some(usage) = model.cores.get(index).copied() else {
        return vec![Span::raw(" ".repeat(bar_width.saturating_add(11)))];
    };
    let (filled, empty) = bar_text(usage.get(), bar_width);
    let heat = heat_of(usage);
    vec![
        Span::styled(format!("C{index:<2} "), styles.secondary),
        Span::styled(filled, styles.bar(heat)),
        Span::styled(empty, styles.border),
        Span::styled(format!(" {:>5}%", usage.to_string()), styles.heat(heat)),
    ]
}

fn percent_text(pressure: Option<Pressure>) -> String {
    pressure.map_or_else(
        || NOT_AVAILABLE.to_owned(),
        |pressure| pressure.some().avg10().to_string(),
    )
}

fn duration_text(seconds: u64) -> String {
    let days = seconds / SECONDS_PER_DAY;
    let hours = seconds % SECONDS_PER_DAY / SECONDS_PER_HOUR;
    let minutes = seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    if days > 0 {
        format!("{days}d {hours:02}h {minutes:02}m")
    } else {
        format!("{hours}h {minutes:02}m")
    }
}

fn draw_memory(frame: &mut Frame, area: Rect, model: &Model, styles: &Styles) {
    let block = panel("Memory · Pressure", styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let bar_width = to_width(inner).saturating_sub(24).clamp(4, 40);
    let mut lines = Vec::with_capacity(5);
    match model.memory {
        Some(memory) => {
            lines.push(usage_line(
                "MEM ",
                memory.used(),
                memory.total(),
                bar_width,
                styles,
            ));
            lines.push(usage_line(
                "SWAP",
                memory.swap_used(),
                memory.swap_total(),
                bar_width,
                styles,
            ));
        }
        None => lines.push(Line::styled("MEM  n/a", styles.secondary)),
    }

    lines.push(Line::from(vec![
        Span::styled("PSI avg10  ", styles.secondary),
        Span::styled(
            format!(
                "cpu {}  mem {}  io {}",
                percent_text(model.pressure.cpu),
                percent_text(model.pressure.memory),
                percent_text(model.pressure.io)
            ),
            styles.accent,
        ),
    ]));

    let load = model.load.map_or_else(
        || NOT_AVAILABLE.to_owned(),
        |load| format!("{} {} {}", load.one(), load.five(), load.fifteen()),
    );
    let uptime = model.uptime.map_or_else(
        || NOT_AVAILABLE.to_owned(),
        |uptime| duration_text(uptime.as_secs()),
    );
    let temperature = model
        .temperature
        .map_or_else(|| NOT_AVAILABLE.to_owned(), |value| value.to_string());
    lines.push(Line::from(vec![
        Span::styled("LOAD ", styles.secondary),
        Span::styled(load, styles.base),
        Span::styled("  UP ", styles.secondary),
        Span::styled(uptime, styles.base),
        Span::styled("  TEMP ", styles.secondary),
        Span::styled(temperature, styles.base),
    ]));
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

fn usage_line<'a>(
    label: &'static str,
    used: borbatop_core::Bytes,
    total: borbatop_core::Bytes,
    bar_width: usize,
    styles: &Styles,
) -> Line<'a> {
    let share = used.permille_of(total);
    let heat = heat_of(Permille::new(share));
    let (filled, empty) = bar_text(share, bar_width);
    Line::from(vec![
        Span::styled(format!("{label} "), styles.secondary),
        Span::styled(filled, styles.bar(heat)),
        Span::styled(empty, styles.border),
        Span::styled(format!(" {used}/{total}"), styles.base),
    ])
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let mut spans = Vec::with_capacity(Tab::ALL.len().saturating_mul(3));
    for tab in Tab::ALL {
        let selected = tab == app.tab;
        let label_style = if selected {
            styles.strong.add_modifier(Modifier::UNDERLINED)
        } else {
            styles.secondary
        };
        spans.push(Span::styled(
            format!(" {}", tab.index().saturating_add(1)),
            styles.accent,
        ));
        spans.push(Span::styled(format!(" {} ", tab.label()), label_style));
        spans.push(Span::styled("│", styles.border));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(styles.base), area);
}

struct Columns {
    unit: bool,
    package: bool,
}

fn columns_for(width: usize, show_package: bool) -> Columns {
    Columns {
        unit: width >= WIDE_WIDTH,
        package: show_package && width >= NARROW_WIDTH,
    }
}

fn process_header(app: &App, columns: &Columns, styles: &Styles) -> Line<'static> {
    let arrow = if app.descending { '▼' } else { '▲' };
    let title = |key: SortKey, text: String| {
        if app.sort == key {
            Span::styled(
                format!("{text}{arrow}"),
                styles.strong.add_modifier(Modifier::UNDERLINED),
            )
        } else {
            Span::styled(text, styles.accent)
        }
    };
    let label = |text: &str, width: usize| Span::styled(fit(text, width), styles.accent);
    let mut spans = vec![
        title(SortKey::Pid, fit_right("PID", PID_WIDTH)),
        Span::raw(" "),
        label("USER", USER_WIDTH),
        Span::raw(" "),
        label("S", STATE_WIDTH),
        title(SortKey::Cpu, fit_right("CPU%", CPU_WIDTH)),
        Span::raw(" "),
        title(SortKey::Memory, fit_right("MEM", MEMORY_WIDTH)),
        Span::raw(" "),
        Span::styled(fit_right("THR", THREADS_WIDTH), styles.accent),
        Span::raw(" "),
    ];
    if columns.unit {
        spans.push(label("UNIT", UNIT_WIDTH));
        spans.push(Span::raw(" "));
    }
    if columns.package {
        spans.push(label("PKG", PACKAGE_WIDTH));
        spans.push(Span::raw(" "));
    }
    spans.push(title(SortKey::Command, "COMMAND".to_owned()));
    Line::from(spans)
}

fn process_line(
    process: &ProcessRow,
    depth: usize,
    columns: &Columns,
    selected: bool,
    styles: &Styles,
    width: usize,
) -> Line<'static> {
    let state = process.state.code();
    let state_style = match state {
        'Z' => styles.danger,
        'R' => styles.accent,
        'D' => styles.base,
        _ => styles.secondary,
    };
    let indent = if depth == 0 {
        String::new()
    } else {
        format!("{}└ ", "  ".repeat(depth.saturating_sub(1)))
    };

    let mut cells = vec![
        (fit_right(&process.pid.to_string(), PID_WIDTH), styles.base),
        (format!(" {} ", fit(&process.user, USER_WIDTH)), styles.base),
        (fit(&state.to_string(), STATE_WIDTH), state_style),
        (
            fit_right(&process.cpu.to_string(), CPU_WIDTH),
            styles.heat(heat_of(process.cpu)),
        ),
        (
            format!(" {}", fit_right(&process.memory.to_string(), MEMORY_WIDTH)),
            styles.base,
        ),
        (
            format!(
                " {} ",
                fit_right(&process.threads.to_string(), THREADS_WIDTH)
            ),
            styles.secondary,
        ),
    ];
    if columns.unit {
        cells.push((
            format!("{} ", fit(&process.unit, UNIT_WIDTH)),
            styles.secondary,
        ));
    }
    if columns.package {
        cells.push((
            format!("{} ", fit(&process.package, PACKAGE_WIDTH)),
            styles.accent,
        ));
    }
    let used: usize = cells.iter().map(|(text, _)| text.chars().count()).sum();
    let command = format!("{indent}{}", process.command);
    cells.push((fit(&command, width.saturating_sub(used)), styles.base));

    Line::from(
        cells
            .into_iter()
            .map(|(text, style)| Span::styled(text, if selected { styles.selected } else { style }))
            .collect::<Vec<_>>(),
    )
}

fn window_start(selected: Option<usize>, total: usize, rows: usize) -> usize {
    let centered = selected.unwrap_or(0).saturating_sub(rows / 2);
    centered.min(total.saturating_sub(rows))
}

fn draw_processes(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let visible = app.visible_processes();
    let title = format!(
        "{} processes{} · {} vanished · {} unreadable",
        visible.len(),
        if app.layout == ProcessLayout::Tree {
            " · tree"
        } else {
            ""
        },
        app.model.vanished,
        app.model.failed
    );
    let block = panel(&title, styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = to_width(inner);
    let columns = columns_for(width, app.package_column == Visibility::Shown);
    let rows = usize::from(inner.height).saturating_sub(1);
    let selected = app.selected_row_position(&visible);
    let start = window_start(selected, visible.len(), rows);

    let mut lines = Vec::with_capacity(rows.saturating_add(1));
    lines.push(process_header(app, &columns, styles));
    for (position, row) in visible.iter().enumerate().skip(start).take(rows) {
        let VisibleRow { index, depth } = *row;
        if let Some(process) = app.model.processes.get(index) {
            let is_selected = selected == Some(position);
            lines.push(process_line(
                process,
                depth,
                &columns,
                is_selected,
                styles,
                width,
            ));
        }
    }
    if visible.is_empty() {
        lines.push(Line::styled(
            "no process matches the filter",
            styles.secondary,
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

fn unit_line(unit: &UnitRow, selected: bool, styles: &Styles, width: usize) -> Line<'static> {
    let (icon, style) = match unit.active() {
        ActiveState::Failed => ("✗", styles.danger),
        ActiveState::Active => ("●", styles.ok),
        ActiveState::Reloading
        | ActiveState::Activating
        | ActiveState::Deactivating
        | ActiveState::Refreshing => ("◐", styles.accent),
        ActiveState::Inactive | ActiveState::Maintenance => ("○", styles.secondary),
    };
    let cells = [
        (format!("{icon} "), style),
        (
            format!("{} ", fit(unit.name(), SERVICE_NAME_WIDTH)),
            styles.base,
        ),
        (
            format!("{} ", fit(unit.active().name(), ACTIVE_WIDTH)),
            style,
        ),
        (format!("{} ", fit(unit.sub(), SUB_WIDTH)), style),
    ];
    let used: usize = cells.iter().map(|(text, _)| text.chars().count()).sum();
    let description = fit(unit.description(), width.saturating_sub(used));

    let mut spans: Vec<Span<'static>> = cells
        .into_iter()
        .map(|(text, cell_style)| {
            Span::styled(
                text,
                if selected {
                    styles.selected
                } else {
                    cell_style
                },
            )
        })
        .collect();
    spans.push(Span::styled(
        description,
        if selected {
            styles.selected
        } else {
            styles.secondary
        },
    ));
    Line::from(spans)
}

fn draw_services(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let units = app.visible_units();
    let failed = units
        .iter()
        .filter(|unit| unit.active() == ActiveState::Failed)
        .count();
    let block = panel(&format!("{} units · {failed} failed", units.len()), styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> =
        if let Err(reason) = &app.model.services {
            vec![Line::styled(format!("systemctl: {reason}"), styles.danger)]
        } else {
            let rows = usize::from(inner.height).saturating_sub(1);
            let selected = units
                .iter()
                .position(|unit| Some(unit.name()) == app.selected_unit.as_deref());
            let start = window_start(selected, units.len(), rows);
            let mut lines = vec![Line::from(Span::styled(
                format!(
                    "  {} {} {} DESCRIPTION",
                    fit("UNIT", SERVICE_NAME_WIDTH),
                    fit("ACTIVE", ACTIVE_WIDTH),
                    fit("SUB", SUB_WIDTH)
                ),
                styles.accent,
            ))];
            lines.extend(units.iter().enumerate().skip(start).take(rows).map(
                |(position, unit)| {
                    unit_line(unit, selected == Some(position), styles, to_width(inner))
                },
            ));
            lines
        };
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

fn section<'a>(title: &str, styles: &Styles) -> Line<'a> {
    Line::styled(title.to_uppercase(), styles.accent)
}

fn error_line<'a>(label: &str, reason: &str, styles: &Styles) -> Line<'a> {
    Line::styled(format!("  {label}: {reason}"), styles.danger)
}

fn pacman_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = upgrade_lines(view, styles);
    lines.extend(kernel_lines(view, styles));
    lines.extend(cleanup_lines(view, styles));
    lines.extend(stale_lines(view, styles));
    lines
}

fn upgrade_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Pending upgrades (synced database)", styles)];
    match &view.upgrades {
        Ok(upgrades) if upgrades.is_empty() => lines.push(Line::styled("  none", styles.ok)),
        Ok(upgrades) => lines.extend(upgrades.iter().map(|upgrade| {
            Line::styled(
                format!(
                    "  {:<32} {} → {}",
                    safe(upgrade.name()),
                    safe(upgrade.from()),
                    safe(upgrade.to())
                ),
                styles.base,
            )
        })),
        Err(reason) => lines.push(error_line("pacman -Qu", reason, styles)),
    }
    lines
}

fn kernel_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Kernel", styles)];
    match &view.kernel {
        Ok(kernel) => {
            lines.push(Line::styled(
                format!("  running {}", safe(&kernel.running)),
                styles.base,
            ));
            lines.push(if kernel.reboot_recommended {
                Line::styled(
                    "  reboot recommended: modules of this kernel are gone",
                    styles.danger,
                )
            } else {
                Line::styled("  up to date", styles.ok)
            });
        }
        Err(reason) => lines.push(error_line("kernel", reason, styles)),
    }
    lines
}

fn cleanup_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Cleanup", styles)];
    match &view.orphans {
        Ok(orphans) if orphans.is_empty() => lines.push(Line::styled("  no orphans", styles.ok)),
        Ok(orphans) => lines.push(Line::styled(
            format!("  {} orphans: {}", orphans.len(), safe(&orphans.join(", "))),
            styles.base,
        )),
        Err(reason) => lines.push(error_line("pacman -Qdtq", reason, styles)),
    }
    match &view.pending_config {
        Ok(files) if files.is_empty() => {
            lines.push(Line::styled("  no .pacnew / .pacsave", styles.ok));
        }
        Ok(files) => lines.extend(
            files
                .iter()
                .map(|file| Line::styled(format!("  {}", safe(file)), styles.danger)),
        ),
        Err(reason) => lines.push(error_line("/etc", reason, styles)),
    }
    match &view.cache {
        Ok(size) => lines.push(Line::styled(format!("  package cache {size}"), styles.base)),
        Err(reason) => lines.push(error_line("cache", reason, styles)),
    }
    lines
}

fn stale_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Processes using replaced libraries", styles)];
    if view.stale.is_empty() {
        lines.push(Line::styled("  none found", styles.ok));
    }
    lines.extend(view.stale.iter().map(|stale| {
        Line::styled(
            format!(
                "  {:>7} {:<16} {}",
                stale.pid,
                safe(&stale.comm),
                safe(&stale.library)
            ),
            styles.base,
        )
    }));
    if view.stale_unreadable > 0 {
        lines.push(Line::styled(
            format!(
                "  {} processes could not be inspected (permissions)",
                view.stale_unreadable
            ),
            styles.secondary,
        ));
    }
    lines
}

fn draw_scrolling(frame: &mut Frame, area: Rect, lines: Vec<Line>, app: &App, styles: &Styles) {
    let height = usize::from(area.height);
    let max_scroll = lines.len().saturating_sub(height);
    app.max_scroll.set(max_scroll);
    let offset = u16::try_from(app.scroll.min(max_scroll)).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(lines).scroll((offset, 0)).style(styles.base),
        area,
    );
}

fn draw_pacman(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let title = if app.is_pacman_loading() {
        "Pacman · loading…"
    } else {
        "Pacman"
    };
    let block = panel(title, styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = match &app.pacman {
        PacmanState::Loaded(view) => pacman_lines(view, styles),
        PacmanState::Loading => vec![Line::styled("reading pacman…", styles.secondary)],
        PacmanState::NotRequested => {
            vec![Line::styled(
                "press R to build the report",
                styles.secondary,
            )]
        }
    };
    draw_scrolling(frame, inner, lines, app, styles);
}

fn rate_text(rate: Option<borbatop_core::BytesPerSecond>) -> String {
    rate.map_or_else(|| NOT_AVAILABLE.to_owned(), |rate| rate.to_string())
}

fn io_lines<'a>(model: &Model, width: usize, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = network_lines(model, width, styles);
    lines.extend(disk_lines(model, styles));
    lines.extend(mount_lines(model, width, styles));
    lines
}

fn network_lines<'a>(model: &Model, width: usize, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Network", styles)];
    let spark_width = width.saturating_sub(40).clamp(8, NET_SPARK_WIDTH);
    for net in &model.network {
        for (label, rate, history) in [
            ("RX", net.received, &net.received_history),
            ("TX", net.sent, &net.sent_history),
        ] {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} {label} ", fit(&net.name, INTERFACE_WIDTH)),
                    styles.secondary,
                ),
                Span::styled(
                    sparkline(history.iter(), scale_of(history.iter()), spark_width),
                    styles.accent,
                ),
                Span::styled(
                    format!(" {}", fit_right(&rate_text(rate), RATE_WIDTH)),
                    styles.base,
                ),
            ]));
        }
    }
    if model.network.is_empty() {
        lines.push(Line::styled("  no interface", styles.secondary));
    }
    lines
}

fn disk_lines<'a>(model: &Model, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Disk I/O", styles)];
    lines.extend(model.disks.iter().map(|disk| {
        Line::from(vec![
            Span::styled(
                format!("  {} ", fit(&disk.name, INTERFACE_WIDTH)),
                styles.secondary,
            ),
            Span::styled(
                format!(
                    "read {}  write {}",
                    fit_right(&rate_text(disk.read), RATE_WIDTH),
                    fit_right(&rate_text(disk.written), RATE_WIDTH)
                ),
                styles.base,
            ),
        ])
    }));
    lines
}

fn mount_lines<'a>(model: &Model, width: usize, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Filesystems", styles)];
    let bar_width = width.saturating_sub(48).clamp(8, 40);
    lines.extend(model.mounts.iter().map(|mount| {
        let share = mount.used.permille_of(mount.total);
        let heat = heat_of(Permille::new(share));
        let (filled, empty) = bar_text(share, bar_width);
        Line::from(vec![
            Span::styled(
                format!("  {} ", fit(&mount.mount_point, MOUNT_WIDTH)),
                styles.base,
            ),
            Span::styled(
                format!("{} ", fit(&mount.fs_type, FS_TYPE_WIDTH)),
                styles.secondary,
            ),
            Span::styled(filled, styles.bar(heat)),
            Span::styled(empty, styles.border),
            Span::styled(
                format!(" {}% {}/{}", Permille::new(share), mount.used, mount.total),
                styles.base,
            ),
        ])
    }));
    lines
}

fn draw_io(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let block = panel("Network · Disk", styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = io_lines(&app.model, to_width(inner), styles);
    draw_scrolling(frame, inner, lines, app, styles);
}

fn key_hint<'a>(key: &'static str, text: &'static str, styles: &Styles) -> [Span<'a>; 2] {
    [
        Span::styled(format!(" {key} "), styles.strong),
        Span::styled(text, styles.secondary),
    ]
}

fn hints_for(tab: Tab) -> &'static [(&'static str, &'static str)] {
    match tab {
        Tab::Processes => &[
            ("↑↓", "move"),
            ("s", "sort"),
            ("r", "reverse"),
            ("/", "filter"),
            ("t", "tree"),
            ("K", "signal"),
            ("p", "pkg"),
            ("␣", "pause"),
            ("q", "quit"),
        ],
        Tab::Services => &[
            ("↑↓", "move"),
            ("/", "filter"),
            ("l", "logs"),
            ("␣", "pause"),
            ("q", "quit"),
        ],
        Tab::Pacman => &[("↑↓", "scroll"), ("R", "refresh"), ("q", "quit")],
        Tab::Io => &[("↑↓", "scroll"), ("␣", "pause"), ("q", "quit")],
    }
}

fn status_line<'a>(app: &App, styles: &Styles) -> Line<'a> {
    if app.input == InputMode::Filtering {
        return Line::from(vec![
            Span::styled(format!("/ {}", safe(&app.filter)), styles.accent),
            Span::styled("█", styles.accent),
            Span::styled("   Enter apply · Esc clear", styles.secondary),
        ]);
    }
    if let Some(message) = &app.message {
        let style = match message.kind {
            MessageKind::Info => styles.base,
            MessageKind::Error => styles.danger,
        };
        return Line::styled(safe(&message.text), style);
    }

    let mut spans: Vec<Span> = Vec::new();
    for (key, text) in hints_for(app.tab) {
        spans.extend(key_hint(key, text, styles));
    }
    if !app.filter.is_empty() {
        spans.push(Span::styled(
            format!("  filter: {}", safe(&app.filter)),
            styles.accent,
        ));
    }
    Line::from(spans)
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    frame.render_widget(
        Paragraph::new(status_line(app, styles)).style(styles.base),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x.saturating_add(area.width.saturating_sub(width) / 2),
        y: area
            .y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    }
}

fn draw_dialog(frame: &mut Frame, dialog: &SignalDialog, styles: &Styles) {
    let area = centered(frame.area(), DIALOG_WIDTH, DIALOG_HEIGHT);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_style(styles.strong)
        .style(styles.base)
        .title(Span::styled(" SEND SIGNAL ", styles.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let target = fit(
        &dialog.command,
        usize::from(inner.width).saturating_sub(DIALOG_RESERVED),
    );
    let mut lines = vec![
        Line::from(vec![
            Span::styled(target, styles.base),
            Span::styled(format!(" PID {}", dialog.id.pid()), styles.secondary),
        ]),
        Line::raw(""),
    ];
    for (position, choice) in SignalChoice::ALL.iter().enumerate() {
        let text = format!(" {:<8} {} ", choice.name(), choice.effect());
        let style = match (position == dialog.selected, *choice == SignalChoice::Kill) {
            (true, true) => styles.danger.add_modifier(Modifier::REVERSED),
            (true, false) => styles.selected,
            (false, true) => styles.danger,
            (false, false) => styles.base,
        };
        lines.push(Line::styled(text, style));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑↓ choose · Enter send · Esc cancel",
        styles.secondary,
    ));
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

fn draw_logs(frame: &mut Frame, logs: &LogView, styles: &Styles) {
    let area = centered(
        frame.area(),
        frame.area().width.saturating_sub(JOURNAL_MARGIN_X),
        frame.area().height.saturating_sub(JOURNAL_MARGIN_Y),
    );
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_style(styles.strong)
        .style(styles.base)
        .title(Span::styled(
            format!(" JOURNAL · {} · Esc to close ", logs.unit),
            styles.accent,
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = usize::from(inner.height);
    let last_top = logs.lines.len().saturating_sub(height);
    let top = last_top.saturating_sub(logs.scroll.min(last_top));
    let style = if logs.failed {
        styles.danger
    } else {
        styles.base
    };
    let lines: Vec<Line> = logs
        .lines
        .iter()
        .skip(top)
        .take(height)
        .map(|line| Line::styled(line.clone(), style))
        .collect();
    frame.render_widget(Paragraph::new(lines).style(styles.base), inner);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use borbatop_core::{
        Bytes, History, MemInfo, Permille, Pid, ProcessState, parse_loadavg, parse_meminfo,
        parse_pressure, parse_systemctl_units,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::UiConfig;
    use crate::input::Key;
    use crate::model::{DiskRow, KernelView, MountRow, NetRow, PacmanView, ProcessRow, StaleView};

    fn row(pid: u32, parent: u32, cpu: u64, command: &str) -> ProcessRow {
        let stat = format!("{pid} (x) S {parent} 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 3 0 1 1 1 0");
        let sample = borbatop_core::parse_process_stat(&stat).unwrap();
        ProcessRow {
            pid: Pid::new(pid),
            start_time: sample.id().start_time(),
            parent: Pid::new(parent),
            user: "borba".to_owned(),
            state: ProcessState::Running,
            cpu: Permille::new(cpu),
            memory: Bytes::new(cpu.saturating_mul(1_000_000)),
            threads: sample.threads(),
            unit: "app-kitty.scope".to_owned(),
            package: "kitty".to_owned(),
            command: command.to_owned(),
        }
    }

    fn meminfo() -> MemInfo {
        parse_meminfo(
            "MemTotal: 16777216 kB\nMemAvailable: 4194304 kB\nSwapTotal: 8388608 kB\nSwapFree: 8388608 kB\n",
        )
        .unwrap()
    }

    fn sample_model() -> Model {
        let mut model = Model::empty(8);
        model.processes = vec![
            row(1, 0, 20, "/usr/lib/systemd/systemd"),
            row(2051, 1, 600, "kitty"),
            row(5120, 2051, 951, "rustc --crate-name borbatop"),
        ];
        model.cores = vec![
            Permille::new(100),
            Permille::new(900),
            Permille::new(450),
            Permille::new(0),
        ];
        model.cpu_history = History::new(8);
        model.cpu_history.push(300);
        model.cpu_history.push(900);
        model.memory = Some(meminfo());
        model.load = Some(parse_loadavg("1.42 1.18 0.97 1/2 3").unwrap());
        model.uptime = Some(std::time::Duration::from_mins(4_572));
        model.pressure.cpu =
            Some(parse_pressure("some avg10=1.20 avg60=0 avg300=0 total=1\n").unwrap());
        model.kernel_release = "7.2.11-arch1-1".to_owned();
        model.services = Ok(parse_systemctl_units(
            "cups.service loaded failed failed CUPS Scheduler\nsshd.service loaded active running OpenSSH\n",
        ));
        model.network = vec![NetRow {
            name: "wlan0".to_owned(),
            received: borbatop_core::bytes_per_second(
                Bytes::new(0),
                Bytes::new(2_048),
                std::time::Duration::from_secs(1),
            ),
            sent: None,
            received_history: History::new(4),
            sent_history: History::new(4),
        }];
        model.disks = vec![DiskRow {
            name: "nvme0n1".to_owned(),
            read: None,
            written: None,
        }];
        model.mounts = vec![MountRow {
            mount_point: "/".to_owned(),
            fs_type: "ext4".to_owned(),
            total: Bytes::new(1_000),
            used: Bytes::new(620),
        }];
        model
    }

    fn app() -> App {
        let mut app = App::new(UiConfig::default());
        app.apply_model(sample_model());
        app
    }

    fn render(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect()
    }

    fn screen(app: &App) -> String {
        render(app, 130, 36).join("\n")
    }

    #[test]
    fn the_logo_is_the_top_left_cell_of_every_tab() {
        let mut app = app();
        for key in ['1', '2', '3', '4'] {
            app.handle_key(Key::Char(key));
            let first_line = render(&app, 130, 36).remove(0);
            assert!(
                first_line.starts_with(&format!("{WORDMARK}.")),
                "tab {key}: {first_line}"
            );
        }
    }

    #[test]
    fn the_logo_dot_is_electric_blue_and_the_wordmark_is_not() {
        let app = app();
        let mut terminal = Terminal::new(TestBackend::new(130, 36)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let dot = &buffer[(5, 0)];
        assert_eq!(dot.symbol(), ".");
        assert_eq!(dot.fg, crate::theme::Theme::borba().accent);
        assert_ne!(buffer[(0, 0)].fg, crate::theme::Theme::borba().accent);
    }

    #[test]
    fn process_screen_shows_panels_columns_and_rows() {
        let text = screen(&app());
        for expected in [
            "CPU · 4 CORES",
            "MEMORY · PRESSURE",
            "Processes",
            "Services",
            "PID",
            "CPU%",
            "rustc --crate-name borbatop",
            "95.1",
            "LOAD 1.42 1.18 0.97",
            "UP 3d 04h 12m",
            "psi",
            "kitty",
        ] {
            assert!(
                text.to_lowercase().contains(&expected.to_lowercase()),
                "missing {expected:?}\n{text}"
            );
        }
    }

    #[test]
    fn busiest_process_is_listed_first_and_selection_is_drawn() {
        let mut app = app();
        app.handle_key(Key::Down);
        let rows = render(&app, 130, 36);
        let header = rows
            .iter()
            .position(|line| line.contains("COMMAND"))
            .unwrap();
        assert!(rows[header + 1].contains("5120"));
        assert!(rows[header + 2].contains("2051"));
    }

    #[test]
    fn narrow_terminals_drop_optional_columns_instead_of_breaking() {
        let rows = render(&app(), 70, 24);
        let text = rows.join("\n");
        assert!(text.contains("COMMAND"));
        assert!(!text.contains("UNIT"));
        assert!(!text.contains("PKG"));
        assert!(rows.iter().all(|line| line.chars().count() == 70));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        for (width, height) in [(1, 1), (10, 3), (20, 8), (40, 10)] {
            let mut app = app();
            for key in ['1', '2', '3', '4', 'K', 'l'] {
                app.handle_key(Key::Char(key));
                let _ = render(&app, width, height);
            }
        }
    }

    #[test]
    fn services_tab_shows_failed_units_first() {
        let mut app = app();
        app.handle_key(Key::Char('2'));
        let text = screen(&app);
        let failed = text.find("cups.service").unwrap();
        let healthy = text.find("sshd.service").unwrap();
        assert!(failed < healthy);
        assert!(text.contains("1 FAILED"));
    }

    #[test]
    fn pacman_tab_shows_the_report_sections() {
        let mut app = app();
        app.handle_key(Key::Char('3'));
        assert!(screen(&app).to_lowercase().contains("reading pacman"));
        app.set_pacman(PacmanView {
            upgrades: Ok(Vec::new()),
            orphans: Ok(vec!["lld".to_owned(), "opencv".to_owned()]),
            kernel: Ok(KernelView {
                running: "7.2.11-arch1-1".to_owned(),
                reboot_recommended: true,
            }),
            pending_config: Ok(vec!["/etc/pacman.d/mirrorlist.pacnew".to_owned()]),
            cache: Ok(Bytes::new(4_508_876_800)),
            stale: vec![StaleView {
                pid: Pid::new(3310),
                comm: "firefox".to_owned(),
                library: "/usr/lib/libssl.so.3".to_owned(),
            }],
            stale_unreadable: 12,
        });
        let text = screen(&app);
        for expected in [
            "reboot recommended",
            "2 orphans: lld, opencv",
            "mirrorlist.pacnew",
            "4.1G",
            "libssl.so.3",
            "12 processes could not be inspected",
        ] {
            assert!(text.contains(expected), "missing {expected:?}\n{text}");
        }
    }

    #[test]
    fn io_tab_shows_network_disks_and_filesystem_fill() {
        let mut app = app();
        app.handle_key(Key::Char('4'));
        let text = screen(&app);
        for expected in ["wlan0", "2.0K/s", "nvme0n1", "62.0%", "ext4"] {
            assert!(text.contains(expected), "missing {expected:?}\n{text}");
        }
    }

    #[test]
    fn signal_dialog_overlays_the_screen() {
        let mut app = app();
        app.handle_key(Key::Down);
        app.handle_key(Key::Char('K'));
        let text = screen(&app);
        for expected in ["SEND SIGNAL", "SIGTERM", "SIGKILL", "end now, no cleanup"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn journal_overlay_shows_the_newest_lines_first_in_view() {
        let mut app = app();
        app.handle_key(Key::Char('2'));
        app.handle_key(Key::Down);
        let lines: Vec<String> = (0..100)
            .map(|number| format!("journal line {number}"))
            .collect();
        app.set_logs("cups.service".to_owned(), Ok(lines.join("\n")));
        let text = screen(&app);
        assert!(text.contains("JOURNAL · cups.service"));
        assert!(text.contains("journal line 99"));
        assert!(!text.contains("journal line 0\n"));
    }

    #[test]
    fn status_bar_reflects_filter_pause_and_messages() {
        let mut app = app();
        app.handle_key(Key::Char(' '));
        assert!(screen(&app).contains("PAUSED"));
        app.handle_key(Key::Char('/'));
        app.handle_key(Key::Char('k'));
        assert!(screen(&app).contains("/ k"));
        app.handle_key(Key::Enter);
        app.notify("kill -15 7 → EPERM", MessageKind::Error);
        assert!(screen(&app).contains("EPERM"));
    }

    #[test]
    fn ansi_theme_renders_the_same_text() {
        let config = UiConfig {
            theme: crate::theme::ThemeKind::Ansi,
            ..UiConfig::default()
        };
        let mut ansi = App::new(config);
        ansi.apply_model(sample_model());
        assert_eq!(screen(&ansi), screen(&app()));
    }

    #[test]
    fn terminal_escape_sequences_in_untrusted_text_never_reach_the_screen() {
        let evil = "\x1b]0;pwned\x07\x1b[2Jevil";
        let mut model = sample_model();
        model.processes.push(row(9, 1, 5, evil));
        model.processes.push(row(10, 1, 4, "tree-child"));
        model.network[0].name = evil.to_owned();
        model.mounts[0].mount_point = evil.to_owned();
        let mut app = App::new(UiConfig::default());
        app.apply_model(model);

        let mut terminal = Terminal::new(TestBackend::new(130, 36)).unwrap();
        for key in ['1', '2', '3', '4'] {
            app.handle_key(Key::Char(key));
            if key == '3' {
                app.set_pacman(PacmanView {
                    upgrades: Ok(Vec::new()),
                    orphans: Ok(vec![evil.to_owned()]),
                    kernel: Ok(KernelView {
                        running: evil.to_owned(),
                        reboot_recommended: false,
                    }),
                    pending_config: Ok(vec![evil.to_owned()]),
                    cache: Ok(Bytes::new(1)),
                    stale: vec![StaleView {
                        pid: Pid::new(9),
                        comm: evil.to_owned(),
                        library: evil.to_owned(),
                    }],
                    stale_unreadable: 0,
                });
            }
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let clean = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| !cell.symbol().chars().any(char::is_control));
            assert!(clean, "tab {key} leaked a control character");
        }

        app.handle_key(Key::Char('2'));
        app.set_logs("x.service".to_owned(), Ok(format!("{evil}\nnormal line")));
        app.handle_key(Key::Char('/'));
        app.handle_key(Key::Char('\x1b'));
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let clean = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| !cell.symbol().chars().any(char::is_control));
        assert!(clean, "journal or filter leaked a control character");
    }

    #[test]
    fn scrolling_clamps_to_the_content_so_the_next_key_moves_at_once() {
        let mut app = app();
        app.handle_key(Key::Char('3'));
        let stale = (0..60)
            .map(|number| StaleView {
                pid: Pid::new(number),
                comm: "proc".to_owned(),
                library: "/usr/lib/libx.so".to_owned(),
            })
            .collect();
        app.set_pacman(PacmanView {
            upgrades: Ok(Vec::new()),
            orphans: Ok(Vec::new()),
            kernel: Ok(KernelView {
                running: "7".to_owned(),
                reboot_recommended: false,
            }),
            pending_config: Ok(Vec::new()),
            cache: Ok(Bytes::new(0)),
            stale,
            stale_unreadable: 0,
        });
        let _ = render(&app, 80, 20);

        app.handle_key(Key::End);
        let bottom = app.scroll;
        assert!(bottom > 0);
        assert_eq!(bottom, app.max_scroll.get());
        app.handle_key(Key::Up);
        assert_eq!(app.scroll, bottom - 1);
        app.handle_key(Key::Home);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn duration_text_formats_days_and_hours() {
        assert_eq!(duration_text(59), "0h 00m");
        assert_eq!(duration_text(3_661), "1h 01m");
        assert_eq!(duration_text(90_000), "1d 01h 00m");
    }
}

//! Rendering: turns the [`App`] state into ratatui widgets.

use borbatop_core::{ActiveState, Permille, Pressure, UnitRow, Upgrade};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{
    App, Fetch, InputMode, LogView, MessageKind, PacmanState, ProcessLayout, Visibility, VisibleRow,
};
use crate::dialog::{Dialog, Tone};
use crate::input::{SortKey, Tab};
use crate::model::{Model, PacmanView, ProcessRow, Report, ReportTone};
use crate::mouse::{HitMap, ListRows, Run};
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
const DIALOG_WIDTH: u16 = 62;
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
const HEADER_INFO_WIDTH: u16 = 48;
const HEADER_LOGO_WIDTH: u16 = 24;
const COMMAND_TITLE_WIDTH: usize = 8;
const SEPARATOR_WIDTH: usize = 1;
const HINT_PADDING: usize = 2;
const ALERT_MARK: &str = "⚠ ";
const ALERT_SEPARATOR: &str = " · ";
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
    *app.hits.borrow_mut() = HitMap::default();

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
        Tab::Connections | Tab::Boot | Tab::Logs | Tab::Gpu | Tab::Containers => {
            draw_report(frame, body, app, &styles);
        }
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
    let [left, middle, right] = Layout::horizontal([
        Constraint::Length(HEADER_LOGO_WIDTH),
        Constraint::Min(0),
        Constraint::Length(HEADER_INFO_WIDTH),
    ])
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

    if let Some(banner) = alert_banner(app, styles) {
        frame.render_widget(Paragraph::new(banner).style(styles.base), middle);
    }

    let kernel = if app.model.kernel_release.is_empty() {
        String::from("linux")
    } else {
        format!("linux {}", app.model.kernel_release)
    };
    let state = match (app.replay, app.is_paused()) {
        (Some((index, total)), true) => Span::styled(
            format!("  ⏸ replay {}/{total}", index.saturating_add(1)),
            styles.danger,
        ),
        (Some((index, total)), false) => Span::styled(
            format!("  ▶ replay {}/{total}", index.saturating_add(1)),
            styles.accent,
        ),
        (None, true) => Span::styled("  ⏸ PAUSED", styles.danger),
        (None, false) => Span::styled("  ● live", styles.ok),
    };
    let info = Line::from(vec![Span::styled(kernel, styles.secondary), state]);
    frame.render_widget(
        Paragraph::new(info)
            .alignment(Alignment::Right)
            .style(styles.base),
        right,
    );
}

/// The alerts as one line for the header, or `None` when everything is calm.
fn alert_banner<'a>(app: &App, styles: &Styles) -> Option<Line<'a>> {
    if app.alerts.is_empty() {
        return None;
    }
    let text = app
        .alerts
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(ALERT_SEPARATOR);
    Some(Line::from(vec![
        Span::styled(ALERT_MARK, styles.danger.add_modifier(Modifier::BOLD)),
        Span::styled(text, styles.danger),
    ]))
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

fn to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let mut spans = Vec::with_capacity(Tab::ALL.len().saturating_mul(3));
    let mut hits = app.hits.borrow_mut();
    hits.tab_row = area.y;
    let mut column = area.x;
    for tab in Tab::ALL {
        let number = format!(" {}", tab.index().saturating_add(1));
        let label = format!(" {} ", tab.label());
        let end = column.saturating_add(to_u16(
            number.chars().count().saturating_add(label.chars().count()),
        ));
        hits.tabs.push(Run {
            start: column,
            end,
            value: tab,
        });
        column = end.saturating_add(to_u16(SEPARATOR_WIDTH));

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

/// Where each sortable column title sits on the header row, in screen columns.
fn sortable_runs(columns: &Columns, left: u16) -> Vec<Run<SortKey>> {
    let mut layout: Vec<(usize, Option<SortKey>)> = vec![
        (PID_WIDTH, Some(SortKey::Pid)),
        (SEPARATOR_WIDTH, None),
        (USER_WIDTH, None),
        (SEPARATOR_WIDTH, None),
        (STATE_WIDTH, None),
        (CPU_WIDTH, Some(SortKey::Cpu)),
        (SEPARATOR_WIDTH, None),
        (MEMORY_WIDTH, Some(SortKey::Memory)),
        (SEPARATOR_WIDTH, None),
        (THREADS_WIDTH, None),
        (SEPARATOR_WIDTH, None),
    ];
    if columns.unit {
        layout.push((UNIT_WIDTH.saturating_add(SEPARATOR_WIDTH), None));
    }
    if columns.package {
        layout.push((PACKAGE_WIDTH.saturating_add(SEPARATOR_WIDTH), None));
    }
    layout.push((COMMAND_TITLE_WIDTH, Some(SortKey::Command)));

    let mut column = left;
    let mut runs = Vec::with_capacity(SortKey::COUNT);
    for (width, key) in layout {
        let end = column.saturating_add(to_u16(width));
        if let Some(value) = key {
            runs.push(Run {
                start: column,
                end,
                value,
            });
        }
        column = end;
    }
    runs
}

/// A column title; the sort arrow of the active column takes a padding cell, never a new one,
/// so the titles stay aligned with the rows below.
fn sort_title(app: &App, key: SortKey, width: usize, styles: &Styles) -> Span<'static> {
    let sorted = app.sort == key;
    let arrow = if app.descending { '▼' } else { '▲' };
    let text = if sorted {
        format!("{}{arrow}", key.label())
    } else {
        key.label().to_owned()
    };
    // Text columns hug the left edge, numeric columns the right edge.
    let cell = match key {
        SortKey::Command => fit(&text, width),
        SortKey::Cpu | SortKey::Memory | SortKey::Pid => fit_right(&text, width),
    };
    if sorted {
        Span::styled(cell, styles.strong.add_modifier(Modifier::UNDERLINED))
    } else {
        Span::styled(cell, styles.accent)
    }
}

fn process_header(app: &App, columns: &Columns, styles: &Styles) -> Line<'static> {
    let label = |text: &str, width: usize| Span::styled(fit(text, width), styles.accent);
    let mut spans = vec![
        sort_title(app, SortKey::Pid, PID_WIDTH, styles),
        Span::raw(" "),
        label("USER", USER_WIDTH),
        Span::raw(" "),
        label("S", STATE_WIDTH),
        sort_title(app, SortKey::Cpu, CPU_WIDTH, styles),
        Span::raw(" "),
        sort_title(app, SortKey::Memory, MEMORY_WIDTH, styles),
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
    spans.push(sort_title(
        app,
        SortKey::Command,
        COMMAND_TITLE_WIDTH,
        styles,
    ));
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
    {
        let mut hits = app.hits.borrow_mut();
        hits.header_row = inner.y;
        hits.sortable = sortable_runs(&columns, inner.x);
        hits.list = Some(ListRows {
            top: inner.y.saturating_add(1),
            first: start,
            shown: visible.len().saturating_sub(start).min(rows),
        });
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
            app.hits.borrow_mut().list = Some(ListRows {
                top: inner.y.saturating_add(1),
                first: start,
                shown: units.len().saturating_sub(start).min(rows),
            });
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

fn upgrade_rows<'a>(upgrades: &[Upgrade], styles: &Styles) -> Vec<Line<'a>> {
    if upgrades.is_empty() {
        return vec![Line::styled("  none", styles.ok)];
    }
    upgrades
        .iter()
        .map(|upgrade| {
            Line::styled(
                format!(
                    "  {:<32} {} → {}",
                    safe(upgrade.name()),
                    safe(upgrade.from()),
                    safe(upgrade.to())
                ),
                styles.base,
            )
        })
        .collect()
}

fn upgrade_lines<'a>(view: &PacmanView, styles: &Styles) -> Vec<Line<'a>> {
    let mut lines = vec![section("Pending upgrades (synced database)", styles)];
    match &view.upgrades {
        Ok(upgrades) => lines.extend(upgrade_rows(upgrades, styles)),
        Err(reason) => lines.push(error_line("pacman -Qu", reason, styles)),
    }
    match &view.aur {
        Ok(None) => {}
        Ok(Some(aur)) => {
            lines.push(section(
                &format!("Pending AUR upgrades ({})", safe(&aur.helper)),
                styles,
            ));
            lines.extend(upgrade_rows(&aur.upgrades, styles));
        }
        Err(reason) => {
            lines.push(section("Pending AUR upgrades", styles));
            lines.push(error_line("AUR helper", reason, styles));
        }
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

fn report_style(tone: ReportTone, styles: &Styles) -> Style {
    match tone {
        ReportTone::Heading => styles.accent,
        ReportTone::Normal => styles.base,
        ReportTone::Dim => styles.secondary,
        ReportTone::Good => styles.ok,
        ReportTone::Bad => styles.danger,
    }
}

fn report_lines<'a>(report: &Report, styles: &Styles) -> Vec<Line<'a>> {
    report
        .lines
        .iter()
        .map(|line| {
            let text = match line.tone {
                ReportTone::Heading => line.text.to_uppercase(),
                ReportTone::Normal | ReportTone::Dim | ReportTone::Good | ReportTone::Bad => {
                    line.text.clone()
                }
            };
            Line::styled(safe(&text), report_style(line.tone, styles))
        })
        .collect()
}

fn draw_report(frame: &mut Frame, area: Rect, app: &App, styles: &Styles) {
    let state = app.reports.get(&app.tab);
    let title = if matches!(state, Some(Fetch::Loading)) {
        format!("{} · loading…", app.tab.label())
    } else {
        app.tab.label().to_owned()
    };
    let block = panel(&title, styles);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = match state {
        Some(Fetch::Loaded(report)) => report_lines(report, styles),
        Some(Fetch::Loading) => vec![Line::styled("reading…", styles.secondary)],
        None => vec![Line::styled(
            "press R to build the report",
            styles.secondary,
        )],
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
            ("⏎", "details"),
            ("s", "sort"),
            ("r", "reverse"),
            ("/", "filter"),
            ("t", "tree"),
            ("K", "signal"),
            ("n", "nice"),
            ("c", "cpus"),
            ("p", "pkg"),
            ("␣", "pause"),
            ("q", "quit"),
        ],
        Tab::Services => &[
            ("↑↓", "move"),
            ("/", "filter"),
            ("l", "logs"),
            ("a", "action"),
            ("␣", "pause"),
            ("q", "quit"),
        ],
        Tab::Pacman => &[("↑↓", "scroll"), ("R", "refresh"), ("q", "quit")],
        Tab::Io => &[("↑↓", "scroll"), ("␣", "pause"), ("q", "quit")],
        Tab::Connections | Tab::Boot | Tab::Logs | Tab::Gpu | Tab::Containers => {
            &[("↑↓", "scroll"), ("R", "refresh"), ("q", "quit")]
        }
    }
}

/// The width a hint takes on the status bar: ` key ` plus its text.
fn hint_width(key: &str, text: &str) -> usize {
    key.chars()
        .count()
        .saturating_add(text.chars().count())
        .saturating_add(HINT_PADDING)
}

/// The hints that fit in `width`, most important first; the last one (quit) is always kept.
fn fitting_hints(
    hints: &'static [(&'static str, &'static str)],
    width: usize,
) -> Vec<(&'static str, &'static str)> {
    let Some((last, leading)) = hints.split_last() else {
        return Vec::new();
    };
    let mut room = width.saturating_sub(hint_width(last.0, last.1));
    let mut kept = Vec::with_capacity(hints.len());
    for &(key, text) in leading {
        let needed = hint_width(key, text);
        if needed > room {
            break;
        }
        room = room.saturating_sub(needed);
        kept.push((key, text));
    }
    kept.push(*last);
    kept
}

fn status_line<'a>(app: &App, width: usize, styles: &Styles) -> Line<'a> {
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
    for (key, text) in fitting_hints(hints_for(app.tab), width) {
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
        Paragraph::new(status_line(app, to_width(area), styles)).style(styles.base),
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

fn tone_style(tone: Tone, styles: &Styles) -> Style {
    match tone {
        Tone::Plain => styles.base,
        Tone::Secondary => styles.secondary,
        Tone::Selected => styles.selected,
        Tone::Danger => styles.danger,
        Tone::DangerSelected => styles.danger.add_modifier(Modifier::REVERSED),
    }
}

fn draw_dialog(frame: &mut Frame, dialog: &Dialog, styles: &Styles) {
    let body_width = DIALOG_WIDTH.saturating_sub(BORDER_ROWS);
    let view = dialog.view(usize::from(body_width));
    let height = u16::try_from(view.lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(BORDER_ROWS);
    let area = centered(frame.area(), DIALOG_WIDTH, height);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_style(styles.strong)
        .style(styles.base)
        .title(Span::styled(format!(" {} ", view.title), styles.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = view
        .lines
        .iter()
        .map(|line| {
            Line::from(
                line.runs
                    .iter()
                    .map(|(text, tone)| Span::styled(text.clone(), tone_style(*tone, styles)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
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
            format!(" {} · Esc to close ", logs.title),
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
    use crate::mouse::Mouse;

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
            nice: sample.nice(),
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
            aur: Ok(None),
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

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn sortable_runs_line_up_with_the_header_text() {
        let app = app();
        let styles = Styles::of(Theme::of(app.config.theme));
        for (unit, package) in [(false, false), (false, true), (true, true)] {
            let columns = Columns { unit, package };
            let header: Vec<char> = line_text(&process_header(&app, &columns, &styles))
                .chars()
                .collect();
            for run in sortable_runs(&columns, 0) {
                let cells: String = header
                    .get(usize::from(run.start)..usize::from(run.end).min(header.len()))
                    .unwrap()
                    .iter()
                    .collect();
                assert!(
                    cells.contains(run.value.label()),
                    "{:?} not under {cells:?}",
                    run.value
                );
            }
        }
    }

    fn click_at(app: &mut App, column: u16, row: u16) -> Option<crate::input::Command> {
        app.handle_mouse(Mouse::Click { column, row })
    }

    #[test]
    fn clicking_a_tab_title_switches_tab() {
        let mut app = app();
        screen(&app);
        let (column, row) = {
            let hits = app.hits.borrow();
            (hits.tabs[1].start + 2, hits.tab_row)
        };
        click_at(&mut app, column, row);
        assert_eq!(app.tab(), Tab::Services);

        screen(&app);
        let (column, row) = {
            let hits = app.hits.borrow();
            (hits.tabs[2].start + 1, hits.tab_row)
        };
        assert_eq!(
            click_at(&mut app, column, row),
            Some(crate::input::Command::RefreshPacman),
            "opening the Pacman tab for the first time builds its report"
        );
    }

    #[test]
    fn clicking_a_column_title_sorts_and_clicking_again_reverses() {
        let mut app = app();
        screen(&app);
        let (column, row) = {
            let hits = app.hits.borrow();
            let memory = hits
                .sortable
                .iter()
                .find(|run| run.value == SortKey::Memory)
                .unwrap();
            (memory.start, hits.header_row)
        };

        click_at(&mut app, column, row);
        assert_eq!(app.sort, SortKey::Memory);
        assert!(app.descending);

        screen(&app);
        click_at(&mut app, column, row);
        assert!(!app.descending);
    }

    #[test]
    fn clicking_a_row_selects_it_and_the_wheel_moves_the_selection() {
        let mut app = app();
        screen(&app);
        let (top, first) = {
            let list = app.hits.borrow().list.unwrap();
            (list.top, list.first)
        };
        let second = app.visible_processes()[first + 1].index;
        let expected = app.model.processes[second].pid;

        click_at(&mut app, 3, top + 1);
        assert_eq!(app.selected_pid, Some(expected));

        let rows = app.visible_processes();
        let last = rows.len() - 1;
        let before = app.selected_row_position(&rows).unwrap();
        app.handle_mouse(Mouse::WheelDown);
        let after = app.selected_row_position(&app.visible_processes()).unwrap();
        assert_eq!(after, (before + 3).min(last));
        app.handle_mouse(Mouse::WheelUp);
        let back = app.selected_row_position(&app.visible_processes()).unwrap();
        assert_eq!(back, after.saturating_sub(3));
    }

    #[test]
    fn clicks_outside_any_target_and_during_dialogs_do_nothing() {
        let mut app = app();
        screen(&app);
        let before = app.selected_pid;
        click_at(&mut app, 0, 0);
        assert_eq!(app.selected_pid, before);

        app.handle_key(Key::Char('K'));
        assert!(app.dialog.is_some());
        let row = app.hits.borrow().list.unwrap().top + 1;
        click_at(&mut app, 3, row);
        assert_eq!(app.selected_pid, before);
        assert!(app.dialog.is_some());
    }

    #[test]
    fn report_tabs_show_loading_then_the_report_with_hostile_text_defused() {
        use crate::model::{Report, ReportLine, ReportTone};

        let mut app = app();
        app.handle_key(Key::Char('7'));
        let loading = screen(&app);
        assert!(loading.contains("LOGS · LOADING"), "{loading}");
        assert!(loading.contains("reading…"));

        app.set_report(
            Tab::Logs,
            Ok(Report {
                lines: vec![
                    ReportLine::new("Errors since boot", ReportTone::Heading),
                    ReportLine::new("kernel: bad \u{1b}[2J thing", ReportTone::Bad),
                ],
            }),
        );
        let text = screen(&app);
        assert!(
            text.contains("ERRORS SINCE BOOT"),
            "headings are upper-cased"
        );
        assert!(text.contains("kernel: bad"));
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains("loading"));
    }

    #[test]
    fn hints_shrink_to_the_width_but_always_keep_quit() {
        let hints = hints_for(Tab::Processes);
        let full = fitting_hints(hints, 500);
        assert_eq!(full.len(), hints.len());

        for width in [10, 40, 60, 80, 100] {
            let kept = fitting_hints(hints, width);
            assert_eq!(kept.last(), Some(&("q", "quit")), "width {width}");
            let used: usize = kept.iter().map(|(key, text)| hint_width(key, text)).sum();
            assert!(
                used <= width.max(hint_width("q", "quit")),
                "width {width}: {used}"
            );
        }
        assert!(fitting_hints(hints, 80).len() < hints.len());
        assert_eq!(fitting_hints(&[], 80), []);
    }

    #[test]
    fn every_tab_has_a_title_a_body_and_hints() {
        let mut app = app();
        for _ in 0..5 {
            app.tick(); // let the "new alert" message fade so the hints show
        }
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            app.handle_key(Key::Char(
                char::from_digit(u32::try_from(index + 1).unwrap(), 10).unwrap(),
            ));
            assert_eq!(app.tab(), tab);
            let text = screen(&app);
            assert!(text.contains("BORBA"), "{tab:?} lost the logo");
            assert!(text.contains("quit"), "{tab:?} lost the hints");
        }
    }

    #[test]
    fn the_process_detail_overlay_uses_its_own_title() {
        let mut app = app();
        app.set_overlay(
            "PROCESS · PID 7 · tool".to_owned(),
            Ok("Executable   /usr/bin/tool\nDescriptors  3 open".to_owned()),
        );
        let text = screen(&app);
        assert!(text.contains("PROCESS · PID 7 · tool"));
        assert!(text.contains("Descriptors  3 open"));
    }

    #[test]
    fn the_header_says_where_a_replay_is() {
        let mut app = App::new(UiConfig::default());
        assert!(screen(&app).contains("● live"));

        app.set_replay(2, 10);
        let playing = screen(&app);
        assert!(playing.contains("▶ replay 3/10"), "{playing}");
        assert!(!playing.contains("live"));

        app.handle_key(Key::Char(' '));
        assert!(screen(&app).contains("⏸ replay 3/10"));
    }

    #[test]
    fn alerts_appear_in_the_header_and_vanish_when_calm() {
        let mut app = App::new(UiConfig::default());
        let calm = screen(&app);
        assert!(!calm.contains('⚠'));

        let mut model = app.model().clone();
        model.cpu_history.push(990);
        app.apply_model(model);
        let text = screen(&app);
        assert!(text.contains("⚠ cpu 99.0%"), "{text}");
    }

    #[test]
    fn nice_and_affinity_dialogs_overlay_the_screen() {
        let mut app = app();
        app.handle_key(Key::Down);
        app.handle_key(Key::Char('n'));
        let text = screen(&app);
        for expected in ["RENICE", "current", "always allowed"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
        app.handle_key(Key::Escape);

        let mut model = app.model().clone();
        model.cores = vec![Permille::new(0); 8];
        app.apply_model(model);
        app.handle_key(Key::Char('c'));
        let text = screen(&app);
        for expected in ["CPU AFFINITY", "[x]0", "[x]7", "Space toggle"] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn service_dialog_overlays_the_services_tab() {
        let mut app = app();
        app.handle_key(Key::Char('2'));
        app.handle_key(Key::Down);
        app.handle_key(Key::Char('a'));
        let text = screen(&app);
        for expected in ["SERVICE", "restart", "stop it now"] {
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
        app.set_logs("cups.service", Ok(lines.join("\n")));
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
                    aur: Ok(None),
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
        app.set_logs("x.service", Ok(format!("{evil}\nnormal line")));
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
            aur: Ok(None),
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

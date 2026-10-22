//! GNOME window: sidebar of areas, a term picker, and resizable panes.

use crate::{load_index, open_target};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use termorganize_core::banner::{self, BannerClient};
use termorganize_core::course_info::CourseInfo;
use termorganize_core::degree_audit::{AuditStatus, DegreeAudit, DegreeCourseAttributes};
use termorganize_core::degree_format;
use termorganize_core::editing;
use termorganize_core::index::CatalogIndex;
use termorganize_core::legal;
use termorganize_core::links::AppLink;
use termorganize_core::model::*;
use termorganize_core::nl;
use termorganize_core::offering::OfferingHistory;
use termorganize_core::planlogic::{self, PrereqStatus};
use termorganize_core::professors;
use termorganize_core::program_parser;
use termorganize_core::rooms;
use termorganize_core::schedule::{self, GeneratedSchedule, ScheduleCourseInput};
use termorganize_core::search::{self, SearchFilters, SearchQuery, SearchSort};
use termorganize_core::store;
use termorganize_core::watchlist;
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

use adw::prelude::*;
use gtk::{gio, glib};

mod gap;
pub(crate) use gap::index_for_code;

const AREAS: &[(&str, &str)] = &[
    ("system-search-symbolic", "Search"),
    ("view-list-symbolic", "Subjects"),
    ("starred-symbolic", "Saved"),
    ("avatar-default-symbolic", "Professors"),
    ("preferences-desktop-screensaver-symbolic", "Free Rooms"),
    ("x-office-calendar-symbolic", "Schedules"),
    ("document-edit-symbolic", "Degree Plans"),
    ("appointment-soon-symbolic", "Watchlist"),
];

struct State {
    index: CatalogIndex,
    data: UserData,
    area: i32,
    query: String,
    filters: SearchFilters,
    sort: SearchSort,
    course: Option<CourseKey>,
    professor: Option<String>,
    subject: Option<String>,
    plan: Option<Uuid>,
    degree: Option<Uuid>,
    generated: Vec<GeneratedSchedule>,
    picked: usize,
    nl: String,
    room_day: i32,
    room_query: String,
    room_date: String,
    requirements: HashMap<Uuid, ProgramRequirements>,
    catalog_path: String,
    live_seats: HashMap<String, (i32, i32, i32)>,
    offering: OfferingHistory,
    course_notes: HashMap<CourseKey, CourseInfo>,
    term_codes: Vec<String>,
    term_labels: Vec<String>,
    /// Unordered pair ids the student turned down, newest last. Same store as the Mac prompt.
    dismissed_pairs: Vec<String>,
    /// Watch items whose last Banner response did not include the CRN.
    missing_watch: HashSet<String>,
    /// `user.json` as it was when this window loaded it, or last saved it.
    stamp: store::FileStamp,
}

/// Which text field should keep the caret after a rebuild.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeepFocus {
    Search,
    Subject,
    Professors,
    RoomBuilding,
    RoomDate,
}

impl KeepFocus {
    fn widget_name(self) -> &'static str {
        match self {
            Self::Search => "to-focus-search",
            Self::Subject => "to-focus-subject",
            Self::Professors => "to-focus-professors",
            Self::RoomBuilding => "to-focus-rooms",
            Self::RoomDate => "to-focus-room-date",
        }
    }
}

struct Shell {
    state: Rc<RefCell<State>>,
    content: gtk::Box,
    toast: adw::ToastOverlay,
    window: adw::ApplicationWindow,
    split: adw::NavigationSplitView,
    title: adw::WindowTitle,
    sidebar: gtk::ListBox,
    guard: Cell<bool>,
    windows: RefCell<Vec<adw::Window>>,
    term_menu: gtk::DropDown,
    keep_focus: Cell<Option<KeepFocus>>,
    /// Divider share captured before an external reload, consumed by the rebuilt page.
    reload_pane_share: Cell<Option<f64>>,
    /// Owns the `user.json` monitor. Dropping it would stop reload events.
    #[allow(dead_code)]
    user_monitor: Option<gio::FileMonitor>,
}

pub(crate) fn course_window_name(key: &CourseKey) -> String {
    format!("course-{}-{}", key.subject, key.number)
}

pub fn run(open: Option<String>, smoke: bool) {
    let app = adw::Application::builder()
        .application_id("org.termorganize.TermOrganize")
        .build();
    let open = Rc::new(open);
    app.connect_startup(install_actions);
    app.connect_activate(move |app| {
        if !app.windows().is_empty() {
            app.windows()[0].present();
            return;
        }
        present(app, (*open).clone(), smoke);
    });
    // GTK would otherwise treat --smoke and other flags as its own options.
    app.run_with_args(&["termorganize"]);
}

fn install_actions(app: &adw::Application) {
    let shortcuts = [
        ("new-schedule", &["<primary>n"][..]),
        ("new-degree", &["<primary><shift>n"]),
        ("import-degree", &["<primary>o"]),
        ("export", &["<primary>e"]),
        ("reload", &["<primary>r"]),
        ("focus-search", &["<primary>f"]),
        ("settings", &["<primary>comma"]),
        ("toggle-sidebar", &["<primary><alt>0", "F9"]),
        ("area0", &["<primary>1"]),
        ("area1", &["<primary>2"]),
        ("area2", &["<primary>3"]),
        ("area3", &["<primary>4"]),
        ("area4", &["<primary>5"]),
        ("area5", &["<primary>6"]),
        ("area6", &["<primary>7"]),
        ("area7", &["<primary>8"]),
        ("copy", &["<primary>c"]),
        ("print", &["<primary>p"]),
        ("quit", &["<primary>q"]),
    ];
    for (name, accels) in shortcuts {
        app.set_accels_for_action(&format!("win.{name}"), accels);
    }
    let menu = gio::Menu::new();
    let file = gio::Menu::new();
    file.append(Some("New Schedule"), Some("win.new-schedule"));
    file.append(Some("New Degree Plan"), Some("win.new-degree"));
    file.append(Some("Import Degree Plan…"), Some("win.import-degree"));
    file.append(Some("Export…"), Some("win.export"));
    file.append(Some("Reload"), Some("win.reload"));
    file.append(Some("Print…"), Some("win.print"));
    file.append(Some("Quit"), Some("win.quit"));
    menu.append_submenu(Some("_File"), &file);
    let view = gio::Menu::new();
    for (i, (_, name)) in AREAS.iter().enumerate() {
        view.append(Some(name), Some(&format!("win.area{i}")));
    }
    view.append(Some("Show or Hide Sidebar"), Some("win.toggle-sidebar"));
    menu.append_submenu(Some("_View"), &view);
    let schedule = gio::Menu::new();
    schedule.append(Some("Generate"), Some("win.generate"));
    schedule.append(Some("Copy CRNs"), Some("win.copy"));
    schedule.append(Some("Export Calendar"), Some("win.export-ics"));
    schedule.append(Some("Export Week Image"), Some("win.export-png"));
    menu.append_submenu(Some("_Schedule"), &schedule);
    let degree = gio::Menu::new();
    degree.append(Some("Add Semester"), Some("win.add-semester"));
    degree.append(Some("Autofill from Catalog"), Some("win.autofill"));
    degree.append(Some("Export PDF"), Some("win.export-pdf"));
    menu.append_submenu(Some("_Degree Plan"), &degree);
    let app_menu = gio::Menu::new();
    app_menu.append(Some("Settings"), Some("win.settings"));
    app_menu.append(Some("About Term Organize"), Some("win.about"));
    menu.append_submenu(Some("_Help"), &app_menu);
    app.set_menubar(Some(&menu));
}

fn present(app: &adw::Application, open: Option<String>, smoke: bool) {
    let Some(index) = load_index() else {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Term Organize")
            .default_width(520)
            .default_height(240)
            .build();
        let label = gtk::Label::new(Some("No catalog snapshot found. Set TERMORGANIZE_DATA to a directory that contains catalog/term-*.catalog, or run Term Organize from the repository."));
        label.set_wrap(true);
        label.set_margin_top(24);
        label.set_margin_bottom(24);
        label.set_margin_start(24);
        label.set_margin_end(24);
        window.set_content(Some(&label));
        window.present();
        return;
    };
    let (mut data, stamp) = store::load_stamped(&store::data_dir());
    if data.preferences.selected_term_code.is_none() {
        data.preferences.selected_term_code = Some(index.catalog.term.code.clone());
    }
    pull_folder(&mut data);
    apply_scheme(&data.preferences);
    let term_name = index.catalog.term.name();
    let state = Rc::new(RefCell::new(State {
        index,
        data,
        area: 0,
        query: String::new(),
        filters: SearchFilters::default(),
        sort: SearchSort::Relevance,
        course: None,
        professor: None,
        subject: None,
        plan: None,
        degree: None,
        generated: vec![],
        picked: 0,
        nl: String::new(),
        room_day: 3,
        room_query: String::new(),
        room_date: "2026-09-16".into(),
        requirements: HashMap::new(),
        catalog_path: termorganize_core::banner::DEFAULT_CATALOG_PATH.into(),
        live_seats: HashMap::new(),
        offering: OfferingHistory::empty(),
        course_notes: HashMap::new(),
        term_codes: vec![],
        term_labels: vec![],
        dismissed_pairs: gap::load_dismissed(),
        missing_watch: HashSet::new(),
        stamp,
    }));
    load_saved_requirements(&state);

    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    for (icon, name) in AREAS {
        sidebar.append(&sidebar_row(icon, name));
    }
    let sidebar_scroll = gtk::ScrolledWindow::builder()
        .child(&sidebar)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    fill(&sidebar_scroll);
    let side_page = adw::NavigationPage::builder()
        .title("Term Organize")
        .child(&sidebar_scroll)
        .build();
    fill(&side_page);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fill(&content);
    let content_page = adw::NavigationPage::builder()
        .title("Search")
        .child(&content)
        .build();
    fill(&content_page);

    let split = adw::NavigationSplitView::new();
    split.set_sidebar(Some(&side_page));
    split.set_content(Some(&content_page));
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(320.0);
    // Without this the split keeps its natural height and the page is a short strip.
    fill(&split);

    let title = adw::WindowTitle::new(
        "Search",
        "Unofficial · not affiliated with Northeastern or Sandbox",
    );
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let term = gtk::DropDown::from_strings(&[term_name.as_str()]);
    term.set_tooltip_text(Some(
        "Term. Cached terms open immediately. Any other Banner term is downloaded and kept.",
    ));
    header.pack_end(&term);
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .build();
    menu_button.set_menu_model(Some(&menu_model()));
    header.pack_end(&menu_button);

    let toast = adw::ToastOverlay::new();
    fill(&toast);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fill(&column);
    column.append(&header);
    column.append(&split);
    toast.set_child(Some(&column));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Term Organize")
        .default_width(1180)
        .default_height(760)
        .width_request(360)
        .height_request(480)
        .content(&toast)
        .show_menubar(true)
        .build();
    window.add_css_class("termorganize");

    let shell = Rc::new(RefCell::new(Shell {
        state,
        content,
        toast: toast.clone(),
        window: window.clone(),
        split: split.clone(),
        title,
        sidebar: sidebar.clone(),
        guard: Cell::new(false),
        windows: RefCell::new(Vec::new()),
        term_menu: term.clone(),
        keep_focus: Cell::new(None),
        reload_pane_share: Cell::new(None),
        user_monitor: None,
    }));
    gap::install_css();
    gap::install_term_menu(&shell, smoke);

    sidebar.connect_row_selected({
        let shell = shell.clone();
        move |_, row| {
            let Some(row) = row else { return };
            if shell.borrow().guard.get() {
                return;
            }
            let index = row.index();
            if index < 0 {
                return;
            }
            {
                let shell = shell.borrow();
                let mut state = shell.state.borrow_mut();
                state.area = index;
                state.course = None;
                state.professor = None;
            }
            refresh(&shell);
        }
    });

    let split_for_size = split.clone();
    window.connect_notify_local(Some("default-width"), move |window, _| {
        let width = window.default_width();
        split_for_size.set_collapsed(width < 860);
    });

    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Px,
    ));
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);
    split.connect_collapsed_notify(|split| {
        if split.is_collapsed() {
            split.set_show_content(true);
        }
    });

    wire_actions(&shell);
    if let Some(row) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&row));
    }
    if let Some(raw) = open.as_deref() {
        if let Some(link) = open_target(raw) {
            apply_link(&shell, link);
        }
    }
    if !shell
        .borrow()
        .state
        .borrow()
        .data
        .preferences
        .has_completed_onboarding
    {
        greet(&shell);
    }
    install_drop(&shell);
    watch_user_file(&shell);
    arm_seat_timer(&shell);
    if !smoke
        && shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .background_refresh_enabled
    {
        request_background();
    }
    if smoke {
        window.set_default_size(1440, 900);
    }
    window.present();
    if smoke {
        let app = app.clone();
        let window = window.clone();
        let content = shell.borrow().content.clone();
        let tries = Rc::new(Cell::new(0));
        glib::timeout_add_local(Duration::from_millis(200), move || {
            tries.set(tries.get() + 1);
            match page_fills_window(&window, &content) {
                Ok(message) => {
                    eprintln!("{message}");
                    app.quit();
                    glib::ControlFlow::Break
                }
                Err(_) if tries.get() < 15 => glib::ControlFlow::Continue,
                Err(message) => {
                    eprintln!("smoke: {message}");
                    std::process::exit(1);
                }
            }
        });
    }
}

fn fill(widget: &impl IsA<gtk::Widget>) {
    widget.set_hexpand(true);
    widget.set_vexpand(true);
}

/// The main page should occupy the window below the menu and header, not a short strip.
fn page_fills_window(
    window: &adw::ApplicationWindow,
    content: &gtk::Box,
) -> Result<String, String> {
    let win_h = window.height();
    let content_h = content.height();
    let page = content.first_child();
    let page_h = page.as_ref().map(|child| child.height()).unwrap_or(0);
    // Widget height excludes CSS margins, which sit inside the content box.
    let margin = page
        .as_ref()
        .map(|child| child.margin_top() + child.margin_bottom())
        .unwrap_or(0);
    if win_h < 700 {
        return Err(format!("window height {win_h} is below 700"));
    }
    if content_h + 160 < win_h {
        return Err(format!(
            "content height {content_h} does not fill window height {win_h}"
        ));
    }
    if page_h + margin + 4 < content_h {
        return Err(format!(
            "page height {page_h} plus margin {margin} does not fill content height {content_h}"
        ));
    }
    Ok(format!("smoke: page fills window (window={win_h} content={content_h} page={page_h} margin={margin})"))
}

fn menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Settings"), Some("win.settings"));
    menu.append(Some("About"), Some("win.about"));
    menu
}

fn wire_actions(shell: &Rc<RefCell<Shell>>) {
    let window = shell.borrow().window.clone();
    let add = |name: &str, shell: &Rc<RefCell<Shell>>, f: fn(&Rc<RefCell<Shell>>)| {
        let action = gio::SimpleAction::new(name, None);
        let shell = shell.clone();
        action.connect_activate(move |_, _| f(&shell));
        window.add_action(&action);
    };
    add("new-schedule", shell, new_schedule);
    add("new-degree", shell, new_degree);
    add("import-degree", shell, import_degree);
    add("export", shell, export_user);
    add("reload", shell, refresh);
    add("focus-search", shell, |s| {
        select_area(s, 0);
    });
    add("settings", shell, open_settings);
    add("about", shell, open_about);
    add("toggle-sidebar", shell, |s| {
        let split = s.borrow().split.clone();
        split.set_show_content(!split.shows_content());
        split.set_collapsed(!split.is_collapsed());
    });
    add("generate", shell, generate);
    add("copy", shell, copy_crns);
    add("export-ics", shell, export_ics);
    add("export-png", shell, export_png);
    add("export-pdf", shell, export_pdf);
    add("print", shell, print_view);
    add("add-semester", shell, add_semester);
    add("autofill", shell, autofill_degree);
    add("quit", shell, |s| s.borrow().window.close());
    for i in 0..8 {
        let action = gio::SimpleAction::new(&format!("area{i}"), None);
        let shell = shell.clone();
        action.connect_activate(move |_, _| select_area(&shell, i));
        window.add_action(&action);
    }
}

fn select_area(shell: &Rc<RefCell<Shell>>, index: i32) {
    let list = shell.borrow().sidebar.clone();
    // Selecting the row that is already selected does not emit row-selected, so a
    // Generate (or a new plan) on the current page would never redraw.
    let already = list.selected_row().is_some_and(|row| row.index() == index);
    if let Some(row) = list.row_at_index(index) {
        list.select_row(Some(&row));
    }
    if already {
        refresh(shell);
    }
}

fn refresh(shell: &Rc<RefCell<Shell>>) {
    let saved_scroll = scroll_values(shell.borrow().content.upcast_ref());
    let area_before = shell.borrow().state.borrow().area;
    let (collapsed, shows_content) = {
        let shell_ref = shell.borrow();
        sidebar_after_reload(
            shell_ref.split.is_collapsed(),
            shell_ref.split.shows_content(),
        )
    };
    let page = build_page(shell);
    fill(&page);
    let shell_ref = shell.borrow();
    while let Some(child) = shell_ref.content.first_child() {
        shell_ref.content.remove(&child);
    }
    shell_ref.content.append(&page);
    let state = shell_ref.state.borrow();
    let (heading, sub) = if let Some(key) = &state.course {
        (
            key.to_string(),
            state
                .index
                .course(key)
                .map(|c| c.title.clone())
                .unwrap_or_default(),
        )
    } else if let Some(name) = &state.professor {
        ("Professors".into(), name.clone())
    } else {
        let name = AREAS
            .get(state.area as usize)
            .map(|(_, n)| *n)
            .unwrap_or("Term Organize");
        let next = up_next_summary(&state);
        let sub = if next.is_empty() {
            format!("{} · unofficial", state.index.catalog.term.name())
        } else {
            format!("{} · {next}", state.index.catalog.term.name())
        };
        (name.into(), sub)
    };
    shell_ref.title.set_title(&heading);
    shell_ref.title.set_subtitle(&sub);
    // Rebuilding the page must not open the navigation sidebar. `default_width`
    // stays at the initial 1180 even after the window is narrowed or the user
    // hides the sidebar, and forcing it open covers the plan list.
    shell_ref.split.set_collapsed(collapsed);
    shell_ref.split.set_show_content(shows_content);
    shell_ref.reload_pane_share.take();
    if let Some(which) = shell_ref.keep_focus.take() {
        restore_focus(&page, which);
    }
    if area_before == shell_ref.state.borrow().area
        && scroll_values(&page).len() == saved_scroll.len()
        && saved_scroll.iter().any(|value| *value > 1.0)
    {
        let page = page.clone();
        let tries = Rc::new(Cell::new(0));
        glib::timeout_add_local(Duration::from_millis(50), move || {
            tries.set(tries.get() + 1);
            // The new page reports a zero scroll range until it is allocated.
            let waiting = scroll_not_ready(&page, &saved_scroll) && tries.get() < 8;
            if waiting {
                glib::ControlFlow::Continue
            } else {
                apply_scroll(&page, &saved_scroll);
                glib::ControlFlow::Break
            }
        });
    }
}

fn scroll_values(root: &gtk::Widget) -> Vec<f64> {
    let mut values = Vec::new();
    collect_scroll(root, &mut values);
    values
}

fn collect_scroll(widget: &gtk::Widget, values: &mut Vec<f64>) {
    if let Some(scrolled) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        values.push(scrolled.vadjustment().value());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_scroll(&current, values);
        child = current.next_sibling();
    }
}

fn scroll_not_ready(root: &gtk::Widget, values: &[f64]) -> bool {
    let mut index = 0;
    let mut waiting = false;
    fn walk(widget: &gtk::Widget, values: &[f64], index: &mut usize, waiting: &mut bool) {
        if let Some(scrolled) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            if let Some(value) = values.get(*index) {
                let adjustment = scrolled.vadjustment();
                let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
                if *value > 1.0 && max + 1.0 < *value {
                    *waiting = true;
                }
            }
            *index += 1;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current, values, index, waiting);
            child = current.next_sibling();
        }
    }
    walk(root, values, &mut index, &mut waiting);
    waiting
}

fn apply_scroll(root: &gtk::Widget, values: &[f64]) {
    let mut index = 0;
    fn walk(widget: &gtk::Widget, values: &[f64], index: &mut usize) {
        if let Some(scrolled) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            if let Some(value) = values.get(*index) {
                let adjustment = scrolled.vadjustment();
                let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
                adjustment.set_value((*value).min(max));
            }
            *index += 1;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current, values, index);
            child = current.next_sibling();
        }
    }
    walk(root, values, &mut index);
}

fn restore_focus(root: &gtk::Widget, which: KeepFocus) {
    let Some(widget) = find_named(root, which.widget_name()) else {
        return;
    };
    glib::idle_add_local_once(move || {
        if widget.parent().is_none() {
            return;
        }
        if !widget.grab_focus() {
            return;
        }
        if let Some(editable) = widget.dynamic_cast_ref::<gtk::Editable>() {
            editable.set_position(-1);
        }
    });
}

fn find_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name().as_str() == name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_named(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn hold_focus(shell: &Rc<RefCell<Shell>>, which: KeepFocus) {
    shell.borrow().keep_focus.set(Some(which));
}

/// Sidebar flags to put back after a reload. The window's default width is not
/// the current width, so a reload must not recompute collapsed from it.
pub(crate) fn sidebar_after_reload(collapsed: bool, shows_content: bool) -> (bool, bool) {
    (collapsed, shows_content)
}

/// Divider share for the page rebuilt by a reload. `None` keeps that page's usual share.
pub(crate) fn pane_share_after_reload(saved: Option<f64>, default_share: f64) -> f64 {
    let Some(share) = saved else {
        return default_share;
    };
    if !share.is_finite() || share < 0.0 {
        return default_share;
    }
    if share == 0.0 {
        return 0.0;
    }
    share.clamp(0.12, 0.7)
}

fn content_pane_share(content: &gtk::Box) -> Option<f64> {
    let paned = content.first_child()?.downcast::<gtk::Paned>().ok()?;
    let width = paned.width();
    if width < 80 {
        return None;
    }
    let share = paned.position() as f64 / width as f64;
    share.is_finite().then_some(share)
}

fn page_pane_share(shell: &Rc<RefCell<Shell>>, default_share: f64) -> f64 {
    pane_share_after_reload(shell.borrow().reload_pane_share.take(), default_share)
}

fn build_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let area = shell.borrow().state.borrow().area;
    let professor = shell.borrow().state.borrow().professor.clone();
    if professor.is_some() && area == 3 {
        return professor_page(shell);
    }
    match area {
        0 => search_page(shell),
        1 => subjects_page(shell),
        2 => saved_page(shell),
        3 => professors_page(shell),
        4 => rooms_page(shell),
        5 => schedules_page(shell),
        6 => degrees_page(shell),
        7 => watch_page(shell),
        _ => search_page(shell),
    }
}

fn search_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let (query, filters, sort, hits) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let response = search::search(
            &SearchQuery {
                text: state.query.clone(),
                filters: state.filters.clone(),
                sort: state.sort,
            },
            &state.index,
        );
        let total = response.results.len();
        let rows: Vec<(CourseKey, String, String, String)> = response
            .results
            .iter()
            .take(250)
            .map(|hit| {
                let open = state.index.stats_for(&hit.course.key());
                (
                    hit.course.key(),
                    hit.course.title.clone(),
                    hit.course.credits_label(),
                    format!("{} open", open.open_section_count),
                )
            })
            .collect();
        (
            state.query.clone(),
            state.filters.clone(),
            state.sort,
            (rows, total, response.crn_match.map(|s| s.crn)),
        )
    };
    let (rows, total, crn) = hits;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Course, professor, CRN, or shorthand like orgo")
        .text(&query)
        .build();
    entry.set_widget_name(KeepFocus::Search.widget_name());
    entry.connect_search_changed({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().to_string();
            if shell.borrow().state.borrow().query == text {
                return;
            }
            shell.borrow().state.borrow_mut().query = text;
            hold_focus(&shell, KeepFocus::Search);
            refresh(&shell);
        }
    });
    root.append(&entry);
    let filters_box = wrapping_box();
    let open_only = gtk::CheckButton::with_label("Open seats");
    open_only.set_active(filters.open_seats_only);
    open_only.connect_toggled({
        let shell = shell.clone();
        move |b| {
            shell.borrow().state.borrow_mut().filters.open_seats_only = b.is_active();
            refresh(&shell);
        }
    });
    let honors = gtk::CheckButton::with_label("Honors");
    honors.set_active(filters.honors_only);
    honors.connect_toggled({
        let shell = shell.clone();
        move |b| {
            shell.borrow().state.borrow_mut().filters.honors_only = b.is_active();
            refresh(&shell);
        }
    });
    let no_prereqs = gtk::CheckButton::with_label("No prerequisites");
    no_prereqs.set_active(filters.no_prerequisites);
    no_prereqs.connect_toggled({
        let shell = shell.clone();
        move |b| {
            shell.borrow().state.borrow_mut().filters.no_prerequisites = b.is_active();
            refresh(&shell);
        }
    });
    let no_coreqs = gtk::CheckButton::with_label("No corequisites");
    no_coreqs.set_active(filters.no_corequisites);
    no_coreqs.connect_toggled({
        let shell = shell.clone();
        move |b| {
            shell.borrow().state.borrow_mut().filters.no_corequisites = b.is_active();
            refresh(&shell);
        }
    });
    let subject = gtk::Entry::builder()
        .placeholder_text("Subject")
        .text(filters.subjects.iter().next().cloned().unwrap_or_default())
        .width_chars(8)
        .build();
    subject.set_widget_name(KeepFocus::Subject.widget_name());
    subject.connect_changed({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().trim().to_ascii_uppercase();
            let mut filters = {
                let shell_ref = shell.borrow();
                let state = shell_ref.state.borrow();
                state.filters.clone()
            };
            filters.subjects.clear();
            if !text.is_empty() {
                filters.subjects.insert(text);
            }
            let unchanged = shell.borrow().state.borrow().filters.subjects == filters.subjects;
            if unchanged {
                return;
            }
            shell.borrow().state.borrow_mut().filters = filters;
            hold_focus(&shell, KeepFocus::Subject);
            refresh(&shell);
        }
    });
    let sort_box =
        gtk::DropDown::from_strings(&["Relevance", "Course number", "Title", "Open seats"]);
    sort_box.set_selected(match sort {
        SearchSort::Relevance => 0,
        SearchSort::CourseNumber => 1,
        SearchSort::Title => 2,
        SearchSort::OpenSeats => 3,
    });
    sort_box.connect_selected_notify({
        let shell = shell.clone();
        move |dd| {
            let sort = match dd.selected() {
                1 => SearchSort::CourseNumber,
                2 => SearchSort::Title,
                3 => SearchSort::OpenSeats,
                _ => SearchSort::Relevance,
            };
            shell.borrow().state.borrow_mut().sort = sort;
            refresh(&shell);
        }
    });
    filters_box.append(&open_only);
    filters_box.append(&honors);
    filters_box.append(&no_prereqs);
    filters_box.append(&no_coreqs);
    filters_box.append(&subject);
    filters_box.append(&sort_box);
    root.append(&filters_box);
    root.append(&search_filter_row(shell, &filters));
    root.append(&column_headers(shell));
    let mut note = format!("Showing {} of {total}", rows.len());
    if let Some(crn) = crn {
        note = format!("CRN {crn}. {note}");
    }
    if filters.active_count() > 0 {
        note = format!("{note} · {} filters", filters.active_count());
    }
    root.append(&dim_label(&note));
    root.append(&course_list(shell, &rows, false));
    root.upcast()
}

fn course_list(
    shell: &Rc<RefCell<Shell>>,
    rows: &[(CourseKey, String, String, String)],
    saved: bool,
) -> gtk::Widget {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let keys: Rc<Vec<CourseKey>> = Rc::new(rows.iter().map(|(key, _, _, _)| key.clone()).collect());
    for (key, title, credits, seats) in rows {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let code = gtk::Label::new(Some(&key.to_string()));
        code.add_css_class("heading");
        code.set_width_chars(12);
        code.set_xalign(0.0);
        let name = gtk::Label::new(Some(title));
        name.set_hexpand(true);
        name.set_xalign(0.0);
        name.set_wrap(true);
        name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        name.set_lines(2);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(48);
        row.append(&code);
        row.append(&name);
        row.append(&meta_label(credits));
        row.append(&meta_label(seats));
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        gap::attach_course_drag(&item, key);
        let mut entries = vec![
            menu_action("Open", {
                let shell = shell.clone();
                let key = key.clone();
                move || gap::open_course_window(&shell, &key)
            }),
            menu_action("Copy code", {
                let shell = shell.clone();
                let code = key.to_string();
                move || copy_text(&shell, &code, "Course code copied")
            }),
        ];
        if saved {
            entries.push(menu_action("Remove", {
                let shell = shell.clone();
                let key = key.clone();
                move || unsave_course(&shell, &key)
            }));
        } else {
            entries.push(menu_action("Save", {
                let shell = shell.clone();
                let key = key.clone();
                move || save_course(&shell, &key)
            }));
        }
        attach_context_menu(&item, entries);
        list.append(&item);
    }
    if rows.is_empty() && !saved {
        let label = gtk::Label::builder()
            .label("Nothing matches. Try a course code, a shorter word, or clear a filter.")
            .margin_top(24)
            .margin_bottom(24)
            .build();
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&label));
        item.set_activatable(false);
        item.set_selectable(false);
        list.append(&item);
    }
    list.connect_row_activated({
        let shell = shell.clone();
        let keys = keys.clone();
        move |_, row| {
            if let Some(key) = keys.get(row.index() as usize) {
                gap::open_course_window(&shell, key);
            }
        }
    });
    if saved {
        let shell = shell.clone();
        let keys = keys.clone();
        attach_delete_key(
            &list,
            Rc::new(move |index| {
                if let Some(key) = keys.get(index as usize) {
                    unsave_course(&shell, key);
                }
            }),
        );
    }
    gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .build()
        .upcast()
}

fn course_page(shell: &Rc<RefCell<Shell>>, key: &CourseKey) -> gtk::Widget {
    let (course, sections) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        (
            state.index.course(key).cloned(),
            state.index.sections_for(key),
        )
    };
    let Some(course) = course else {
        return gtk::Label::new(Some("That course isn’t in this term.")).upcast();
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    root.set_margin_bottom(18);
    let bar = wrapping_box();
    let save = gtk::Button::with_label("Save");
    save.connect_clicked({
        let shell = shell.clone();
        let key = key.clone();
        move |_| save_course(&shell, &key)
    });
    let refresh_seats = gtk::Button::with_label("Refresh seats");
    refresh_seats.connect_clicked({
        let shell = shell.clone();
        let key = key.clone();
        move |_| refresh_live_seats(&shell, &key)
    });
    let add = gtk::Button::with_label("Add to schedule");
    add.connect_clicked({
        let shell = shell.clone();
        let key = key.clone();
        move |_| add_course_to_plan(&shell, &key)
    });
    bar.append(&save);
    bar.append(&add);
    bar.append(&refresh_seats);
    root.append(&bar);
    let heading = gtk::Label::new(Some(&format!("{}  {}", course.code(), course.title)));
    heading.add_css_class("title-1");
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    root.append(&heading);
    root.append(&dim_label(&format!(
        "{} · {}",
        course.credits_label(),
        course
            .nupaths()
            .iter()
            .map(|p| p.short())
            .collect::<Vec<_>>()
            .join(" ")
    )));
    if !course.description.is_empty() {
        let body = gtk::Label::new(Some(&course.description));
        body.set_wrap(true);
        body.set_xalign(0.0);
        body.set_selectable(true);
        root.append(&body);
    }
    if let Some(prereq) = &course.prereqs {
        root.append(&heading_label("Prerequisites"));
        root.append(&gap::requisite_block(shell, prereq, "Prerequisites"));
    }
    if let Some(coreq) = &course.coreqs {
        root.append(&heading_label("Corequisites"));
        root.append(&gap::requisite_block(shell, coreq, "Corequisites"));
    }
    if !course.postreqs.is_empty() {
        root.append(&heading_label("Leads to"));
        root.append(&selectable(
            &course
                .postreqs
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    root.append(&heading_label(&format!("{} sections", sections.len())));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let mut crns = Vec::new();
    for section in &sections {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&gtk::Label::new(Some(&format!(
            "CRN {} · {}",
            section.crn, section.section_number
        ))));
        let (seats_n, wait_n) = shell
            .borrow()
            .state
            .borrow()
            .live_seats
            .get(&section.crn)
            .copied()
            .map(|(s, w, _)| (s, w))
            .unwrap_or((section.seats_remaining, section.waitlist_remaining));
        let seats = gtk::Label::new(Some(&seat_wait_label(seats_n, wait_n)));
        seats.set_hexpand(true);
        seats.set_xalign(1.0);
        top.append(&seats);
        row.append(&top);
        let detail = dim_label(&format!(
            "{} · {} · {}",
            section.schedule_summary(),
            section.instructor_label(),
            section.campus
        ));
        detail.set_wrap(true);
        detail.set_max_width_chars(42);
        row.append(&detail);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let copy = gtk::Button::with_label("Copy CRN");
        let crn = section.crn.clone();
        let shell_copy = shell.clone();
        copy.connect_clicked(move |_| {
            shell_copy.borrow().window.clipboard().set_text(&crn);
            toast(&shell_copy, "CRN copied");
        });
        let watch_button = gtk::Button::with_label("Watch");
        let shell_watch = shell.clone();
        let section_watch = section.clone();
        let course_title = course.title.clone();
        watch_button
            .connect_clicked(move |_| watch_section(&shell_watch, &section_watch, &course_title));
        buttons.append(&copy);
        buttons.append(&watch_button);
        row.append(&buttons);
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        let crn_menu = section.crn.clone();
        let section_menu = section.clone();
        let title_menu = course.title.clone();
        attach_context_menu(
            &item,
            vec![
                menu_action("Copy CRN", {
                    let shell = shell.clone();
                    let crn = crn_menu.clone();
                    move || copy_text(&shell, &crn, "CRN copied")
                }),
                menu_action("Watch", {
                    let shell = shell.clone();
                    move || watch_section(&shell, &section_menu, &title_menu)
                }),
            ],
        );
        crns.push(crn_menu);
        list.append(&item);
    }
    let crns = Rc::new(crns);
    list.connect_row_activated({
        let shell = shell.clone();
        move |_, row| {
            if let Some(crn) = crns.get(row.index() as usize) {
                copy_text(&shell, crn, "CRN copied");
            }
        }
    });
    root.append(&list);
    root.upcast()
}

fn subjects_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let (subjects, selected, courses) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let subjects: Vec<(String, String)> = state
            .index
            .active_subjects
            .iter()
            .map(|s| (s.code.clone(), s.name.clone()))
            .collect();
        let selected = state
            .subject
            .clone()
            .or_else(|| subjects.first().map(|(c, _)| c.clone()));
        let mut courses = if let Some(code) = &selected {
            state
                .index
                .courses
                .iter()
                .filter(|c| &c.subject == code)
                .map(|c| (c.key(), c.title.clone(), c.credits_label(), String::new()))
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        match state.sort {
            SearchSort::Title => courses.sort_by(|a, b| {
                a.1.to_ascii_lowercase()
                    .cmp(&b.1.to_ascii_lowercase())
                    .then(a.0.cmp(&b.0))
            }),
            SearchSort::CourseNumber | SearchSort::Relevance | SearchSort::OpenSeats => {
                courses.sort_by(|a, b| a.0.cmp(&b.0))
            }
        }
        courses.truncate(400);
        (subjects, selected, courses)
    };
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    install_proportional_pane(&paned, page_pane_share(shell, 0.32));
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    for (code, name) in &subjects {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        row.set_hexpand(true);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let code_label = gtk::Label::new(Some(code));
        code_label.set_xalign(0.0);
        code_label.add_css_class("heading");
        let name_label = dim_label(&soft_wrap(name, 32));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_wrap(true);
        name_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        name_label.set_tooltip_text(Some(name));
        row.append(&code_label);
        row.append(&name_label);
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        list.append(&item);
    }
    if let Some(selected) = &selected {
        if let Some(i) = subjects.iter().position(|(c, _)| c == selected) {
            if let Some(row) = list.row_at_index(i as i32) {
                list.select_row(Some(&row));
            }
        }
    }
    list.connect_row_selected({
        let shell = shell.clone();
        let subjects = subjects.clone();
        move |_, row| {
            let Some(row) = row else { return };
            let index = row.index();
            if index < 0 {
                return;
            }
            if let Some((code, _)) = subjects.get(index as usize) {
                let current = shell.borrow().state.borrow().subject.clone();
                if current.as_deref() == Some(code.as_str()) {
                    return;
                }
                shell.borrow().state.borrow_mut().subject = Some(code.clone());
                refresh(&shell);
            }
        }
    });
    let subject_scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .build();
    paned.set_start_child(Some(&subject_scroll));
    let subject_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    subject_column.set_margin_top(12);
    subject_column.set_margin_bottom(12);
    subject_column.set_margin_start(12);
    subject_column.set_margin_end(12);
    subject_column.append(&column_headers(shell));
    subject_column.append(&course_list(shell, &courses, false));
    paned.set_end_child(Some(&subject_column));
    paned.upcast()
}

fn saved_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let rows = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let term = state.index.catalog.term.code.clone();
        state
            .data
            .saved
            .iter()
            .filter(|s| s.term_code == term)
            .filter_map(|s| {
                state
                    .index
                    .course(&s.key)
                    .map(|c| (c.key(), c.title.clone(), c.credits_label(), "Saved".into()))
            })
            .collect::<Vec<_>>()
    };
    let root = page_column(
        "Saved courses",
        "Courses you starred in this term. Delete removes one from the list.",
    );
    if rows.is_empty() {
        let status = adw::StatusPage::new();
        status.set_icon_name(Some("starred-symbolic"));
        status.set_title("Nothing saved yet");
        status.set_description(Some("Open a course and choose Save."));
        status.set_vexpand(true);
        root.append(&status);
        return root.upcast();
    }
    root.append(&course_list(shell, &rows, true));
    root.upcast()
}

fn professors_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let (query, people) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let q = state.query.to_ascii_lowercase();
        let people: Vec<(String, String, usize)> = state
            .index
            .instructors
            .iter()
            .filter(|i| {
                q.is_empty()
                    || i.display_name.to_ascii_lowercase().contains(&q)
                    || i.name.to_ascii_lowercase().contains(&q)
            })
            .take(200)
            .map(|i| (i.name.clone(), i.display_name.clone(), i.course_keys.len()))
            .collect();
        (state.query.clone(), people)
    };
    let root = page_column("Professors", "Everyone teaching a section this term.");
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Name")
        .text(&query)
        .build();
    entry.set_widget_name(KeepFocus::Professors.widget_name());
    entry.connect_search_changed({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().to_string();
            if shell.borrow().state.borrow().query == text {
                return;
            }
            shell.borrow().state.borrow_mut().query = text;
            hold_focus(&shell, KeepFocus::Professors);
            refresh(&shell);
        }
    });
    root.append(&entry);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let names: Rc<Vec<String>> = Rc::new(people.iter().map(|(name, _, _)| name.clone()).collect());
    for (name, display, count) in &people {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let label = gtk::Label::new(Some(display));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row.append(&label);
        row.append(&dim_label(&format!("{count} courses")));
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        attach_context_menu(
            &item,
            vec![
                menu_action("Open", {
                    let shell = shell.clone();
                    let name = name.clone();
                    move || gap::open_professor_window(&shell, &name)
                }),
                menu_action("Copy name", {
                    let shell = shell.clone();
                    let display = display.clone();
                    move || copy_text(&shell, &display, "Name copied")
                }),
            ],
        );
        list.append(&item);
    }
    list.connect_row_activated({
        let shell = shell.clone();
        let names = names.clone();
        move |_, row| {
            if let Some(name) = names.get(row.index() as usize) {
                gap::open_professor_window(&shell, name);
            }
        }
    });
    root.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build(),
    );
    root.upcast()
}

fn professor_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let name = shell
        .borrow()
        .state
        .borrow()
        .professor
        .clone()
        .unwrap_or_default();
    let page = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        professors::page(&state.index, &name)
    };
    let root = page_column("Professor", &name);
    let back = gtk::Button::with_label("All professors");
    back.set_halign(gtk::Align::Start);
    back.connect_clicked({
        let shell = shell.clone();
        move |_| {
            shell.borrow().state.borrow_mut().professor = None;
            refresh(&shell);
        }
    });
    root.append(&back);
    let Some(page) = page else {
        root.append(&dim_label("No sections under that name."));
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(720);
        clamp.set_tightening_threshold(480);
        clamp.set_child(Some(&root));
        return gtk::ScrolledWindow::builder()
            .child(&clamp)
            .vexpand(true)
            .build()
            .upcast();
    };
    root.append(&heading_label(&page.instructor.display_name));
    for course in &page.courses {
        let group = adw::PreferencesGroup::new();
        group.set_title(&format!("{} · {}", course.key, course.title));
        for section in &course.sections {
            let row = adw::ActionRow::new();
            row.set_title(&format!("CRN {}", section.crn));
            row.set_subtitle(&format!(
                "{} · {}",
                section.schedule_summary(),
                section.seats_label()
            ));
            row.set_subtitle_lines(2);
            group.add(&row);
        }
        root.append(&group);
    }
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(720);
    clamp.set_tightening_threshold(480);
    clamp.set_child(Some(&root));
    gtk::ScrolledWindow::builder()
        .child(&clamp)
        .vexpand(true)
        .build()
        .upcast()
}

struct RoomBar {
    label: String,
    status: String,
    free: bool,
    free_spans: Vec<(i32, i32, bool)>,
    classes: Vec<(i32, i32, String)>,
}

struct RoomCanvas {
    dark: bool,
    marker: Option<i32>,
    rows: Vec<RoomBar>,
}

fn room_bars(shell: &Rc<RefCell<Shell>>) -> (i32, String, String, Vec<RoomBar>, Option<i32>) {
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let day = state.room_day;
    let date = state.room_date.clone();
    let marker = today_marker(&date);
    let moment = marker.unwrap_or(rooms::Moment::FIRST_CLASS);
    let mut bars = Vec::new();
    for room in rooms::search_rooms(&state.index.buildings, &state.room_query)
        .into_iter()
        .take(40)
    {
        let bookings = rooms::bookings_of(&room, day, Some(date.as_str()));
        let status = rooms::status_of_bookings(&bookings, moment);
        let stretches = rooms::free_stretches(
            &bookings,
            rooms::Moment::FIRST_CLASS,
            rooms::Moment::LAST_CLASS,
        );
        bars.push(RoomBar {
            label: room.label(),
            status: status.summary(),
            free: status.is_free(),
            free_spans: split_free(&stretches, marker),
            classes: bookings
                .iter()
                .map(|booking| (booking.start, booking.end, booking.course.to_string()))
                .collect(),
        });
    }
    bars.sort_by(|left, right| {
        right
            .free
            .cmp(&left.free)
            .then_with(|| left.label.cmp(&right.label))
    });
    (day, state.room_query.clone(), date, bars, marker)
}

fn today_marker(date: &str) -> Option<i32> {
    let now = glib::DateTime::now_local().ok()?;
    let today = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day_of_month()
    );
    if date != today {
        return None;
    }
    let minutes = now.hour() * 60 + now.minute();
    (rooms::Moment::FIRST_CLASS..rooms::Moment::LAST_CLASS)
        .contains(&minutes)
        .then_some(minutes)
}

fn split_free(stretches: &[std::ops::Range<i32>], marker: Option<i32>) -> Vec<(i32, i32, bool)> {
    let mut out = Vec::new();
    for stretch in stretches {
        let Some(marker) = marker else {
            out.push((stretch.start, stretch.end, false));
            continue;
        };
        if marker <= stretch.start {
            out.push((stretch.start, stretch.end, false));
        } else if marker >= stretch.end {
            out.push((stretch.start, stretch.end, true));
        } else {
            out.push((stretch.start, marker, true));
            out.push((marker, stretch.end, false));
        }
    }
    out
}

fn rooms_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let (day, query, date, bars, marker) = room_bars(shell);
    let root = page_column("Free rooms", "Each bar is one room from 7:00 AM to 10:00 PM. Green is free. Gray is a class. A blue line marks the current time when the date is today.");
    let controls = wrapping_box();
    let days = gtk::DropDown::from_strings(&[
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ]);
    days.set_selected(day as u32);
    days.connect_selected_notify({
        let shell = shell.clone();
        move |dd| {
            shell.borrow().state.borrow_mut().room_day = dd.selected() as i32;
            refresh(&shell);
        }
    });
    let date_entry = gtk::Entry::builder()
        .text(&date)
        .width_chars(12)
        .tooltip_text("Date, YYYY-MM-DD")
        .build();
    date_entry.set_widget_name(KeepFocus::RoomDate.widget_name());
    date_entry.connect_changed({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().to_string();
            if shell.borrow().state.borrow().room_date == text {
                return;
            }
            shell.borrow().state.borrow_mut().room_date = text;
            hold_focus(&shell, KeepFocus::RoomDate);
            refresh(&shell);
        }
    });
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Building, like WVH")
        .text(&query)
        .build();
    search.set_widget_name(KeepFocus::RoomBuilding.widget_name());
    search.connect_search_changed({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().to_string();
            if shell.borrow().state.borrow().room_query == text {
                return;
            }
            shell.borrow().state.borrow_mut().room_query = text;
            hold_focus(&shell, KeepFocus::RoomBuilding);
            refresh(&shell);
        }
    });
    controls.append(&days);
    controls.append(&date_entry);
    controls.append(&search);
    root.append(&controls);
    if bars.is_empty() {
        root.append(&dim_label("No rooms match. Try another building code."));
    } else {
        let drawing = gtk::DrawingArea::new();
        let height = 36 + bars.len() as i32 * 36;
        drawing.set_content_height(height);
        drawing.set_hexpand(true);
        drawing.set_vexpand(true);
        let dark = adw::StyleManager::default().is_dark();
        let view = RoomCanvas {
            dark,
            marker,
            rows: bars,
        };
        drawing.set_draw_func(move |_, cr, width, height| draw_rooms(cr, width, height, &view));
        root.append(
            &gtk::ScrolledWindow::builder()
                .child(&drawing)
                .vexpand(true)
                .build(),
        );
    }
    root.upcast()
}

fn schedules_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let snapshot = schedule_snapshot(shell);
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    install_proportional_pane(&paned, page_pane_share(shell, 0.26));
    let side = gtk::Box::new(gtk::Orientation::Vertical, 6);
    side.set_margin_top(8);
    side.set_margin_start(8);
    side.set_margin_end(8);
    let new_button = gtk::Button::with_label("New schedule");
    new_button.connect_clicked({
        let shell = shell.clone();
        move |_| new_schedule(&shell)
    });
    side.append(&new_button);
    let plans = gtk::ListBox::new();
    plans.add_css_class("navigation-sidebar");
    for (i, (id, name, count, primary)) in snapshot.plans.iter().enumerate() {
        let label = gtk::Label::new(Some(&format!(
            "{}{}",
            name,
            if *primary { " · primary" } else { "" }
        )));
        label.set_xalign(0.0);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        label.set_margin_start(8);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        let plan_id = *id;
        attach_context_menu(
            &row,
            vec![menu_action("Delete", {
                let shell = shell.clone();
                move || delete_schedule(&shell, plan_id)
            })],
        );
        plans.append(&row);
        if snapshot.plan.as_ref().map(|p| p.id) == Some(*id) {
            plans.select_row(Some(&row));
        }
        let _ = (i, count);
    }
    {
        let ids: Rc<Vec<Uuid>> = Rc::new(snapshot.plans.iter().map(|(id, _, _, _)| *id).collect());
        let shell = shell.clone();
        attach_delete_key(
            &plans,
            Rc::new(move |index| {
                if let Some(id) = ids.get(index as usize) {
                    delete_schedule(&shell, *id);
                }
            }),
        );
    }
    plans.connect_row_selected({
        let shell = shell.clone();
        let ids: Vec<Uuid> = snapshot.plans.iter().map(|(id, _, _, _)| *id).collect();
        move |_, row| {
            let Some(row) = row else { return };
            let index = row.index();
            if index < 0 {
                return;
            }
            if let Some(id) = ids.get(index as usize) {
                let current = shell.borrow().state.borrow().plan;
                if current == Some(*id) {
                    return;
                }
                shell.borrow().state.borrow_mut().plan = Some(*id);
                shell.borrow().state.borrow_mut().generated.clear();
                refresh(&shell);
            }
        }
    });
    side.append(
        &gtk::ScrolledWindow::builder()
            .child(&plans)
            .vexpand(true)
            .build(),
    );
    paned.set_start_child(Some(&side));

    let main = gtk::Box::new(gtk::Orientation::Vertical, 12);
    main.set_margin_top(12);
    main.set_margin_bottom(18);
    main.set_margin_start(12);
    main.set_margin_end(12);
    if snapshot.plan.is_none() {
        main.append(&dim_label(
            "Make a schedule for this term, then add courses from search or a degree plan.",
        ));
        paned.set_end_child(Some(&main));
        return paned.upcast();
    }
    let toolbar = wrapping_box();
    for (label, action) in [
        ("Generate", 0),
        ("Copy CRNs", 1),
        ("Export .ics", 2),
        ("Export image", 3),
        ("Autofill", 4),
    ] {
        let button = gtk::Button::with_label(label);
        let shell = shell.clone();
        button.connect_clicked(move |_| match action {
            0 => generate(&shell),
            1 => copy_crns(&shell),
            2 => export_ics(&shell),
            3 => export_png(&shell),
            _ => autofill_schedule(&shell),
        });
        toolbar.append(&button);
    }
    main.append(&toolbar);
    main.append(&heading_label("Courses"));
    main.append(&dim_label(
        "Drop a course here from search or a degree plan. Return opens it. Delete removes it.",
    ));
    let course_list = gtk::ListBox::new();
    course_list.add_css_class("boxed-list");
    course_list.set_selection_mode(gtk::SelectionMode::Single);
    course_list.set_activate_on_single_click(false);
    attach_schedule_drop(&course_list, shell);
    let course_keys: Rc<Vec<String>> = Rc::new(
        snapshot
            .courses
            .iter()
            .map(|(key, _)| key.clone())
            .collect(),
    );
    for (key, required) in &snapshot.courses {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let label = gtk::Label::new(Some(&format!(
            "{}{}",
            key,
            if *required { "" } else { " · optional" }
        )));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row.append(&label);
        let remove = gtk::Button::with_label("Remove");
        let shell_remove = shell.clone();
        let key_remove = key.clone();
        remove.connect_clicked(move |_| remove_plan_course(&shell_remove, &key_remove));
        row.append(&remove);
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        let key_menu = key.clone();
        attach_context_menu(
            &item,
            vec![
                menu_action("Open", {
                    let shell = shell.clone();
                    let key = key_menu.clone();
                    move || {
                        if let Some(parsed) = CourseKey::parse(&key) {
                            gap::open_course_window(&shell, &parsed);
                        }
                    }
                }),
                menu_action("Copy code", {
                    let shell = shell.clone();
                    let key = key_menu.clone();
                    move || copy_text(&shell, &key, "Course code copied")
                }),
                menu_action("Remove", {
                    let shell = shell.clone();
                    let key = key_menu;
                    move || remove_plan_course(&shell, &key)
                }),
            ],
        );
        course_list.append(&item);
    }
    course_list.connect_row_activated({
        let shell = shell.clone();
        let course_keys = course_keys.clone();
        move |_, row| {
            if let Some(key) = course_keys
                .get(row.index() as usize)
                .and_then(|key| CourseKey::parse(key))
            {
                gap::open_course_window(&shell, &key);
            }
        }
    });
    attach_delete_key(&course_list, {
        let shell = shell.clone();
        Rc::new(move |index| {
            if let Some(key) = course_keys.get(index as usize) {
                remove_plan_course(&shell, key);
            }
        })
    });
    attach_schedule_drop(&main, shell);
    main.append(&course_list);
    let add = gtk::Entry::builder()
        .placeholder_text("Add a code, like CS 3000")
        .build();
    add.connect_activate({
        let shell = shell.clone();
        move |entry| {
            if let Some(key) = CourseKey::parse(&entry.text()) {
                add_course_to_plan(&shell, &key);
                entry.set_text("");
            }
        }
    });
    main.append(&add);
    main.append(&gap::preference_survey(shell));
    if !snapshot.pref_summary.is_empty() {
        main.append(&selectable(&snapshot.pref_summary));
    }
    if snapshot.generated.is_empty() {
        main.append(&dim_label(
            "Generate builds conflict-free combinations and ranks them.",
        ));
    } else {
        main.append(&heading_label(&format!(
            "{} schedules",
            snapshot.total_label
        )));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for (i, item) in snapshot.generated.iter().enumerate() {
            let text = format!("{}. {}  ·  {}", i + 1, item.crns, item.reasons);
            let label = gtk::Label::new(Some(&text));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(8);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            list.append(&row);
            if i == snapshot.picked {
                list.select_row(Some(&row));
            }
        }
        list.connect_row_selected({
            let shell = shell.clone();
            move |_, row| {
                let Some(row) = row else { return };
                let index = row.index();
                if index < 0 {
                    return;
                }
                shell.borrow().state.borrow_mut().picked = index as usize;
                refresh(&shell);
            }
        });
        main.append(
            &gtk::ScrolledWindow::builder()
                .child(&list)
                .min_content_height(140)
                .build(),
        );
    }
    if !snapshot.suggestions.is_empty() {
        main.append(&heading_label("Nothing fit. Try loosening one rule."));
        for text in &snapshot.suggestions {
            main.append(&dim_label(text));
        }
    }
    let drawing = gtk::DrawingArea::new();
    drawing.set_content_height(360);
    drawing.set_hexpand(true);
    let blocks = snapshot.blocks.clone();
    let show_weekends = snapshot.show_weekends;
    let start_hour = snapshot.start_hour;
    let end_hour = snapshot.end_hour;
    let dark = adw::StyleManager::default().is_dark();
    let view = WeekView {
        dark,
        show_weekends,
        start_hour,
        end_hour,
    };
    drawing.set_draw_func(move |_, cr, width, height| draw_week(cr, width, height, &blocks, &view));
    main.append(&drawing);
    let scroll = gtk::ScrolledWindow::builder().child(&main).build();
    paned.set_end_child(Some(&scroll));
    paned.upcast()
}

struct ScheduleSnap {
    plans: Vec<(Uuid, String, usize, bool)>,
    plan: Option<SchedulePlan>,
    courses: Vec<(String, bool)>,
    pref_summary: String,
    generated: Vec<SchedLine>,
    picked: usize,
    total_label: String,
    suggestions: Vec<String>,
    blocks: Vec<WeekBlock>,
    show_weekends: bool,
    start_hour: i32,
    end_hour: i32,
}

struct SchedLine {
    crns: String,
    reasons: String,
}

#[derive(Clone)]
struct WeekBlock {
    day: i32,
    start: i32,
    end: i32,
    label: String,
    color: usize,
}

fn schedule_snapshot(shell: &Rc<RefCell<Shell>>) -> ScheduleSnap {
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let term = state.index.catalog.term.code.clone();
    let plans = state
        .data
        .plans
        .iter()
        .filter(|p| p.term_code == term)
        .map(|p| (p.id, p.name.clone(), p.courses.len(), p.is_primary))
        .collect::<Vec<_>>();
    let plan = state
        .plan
        .and_then(|id| state.data.plans.iter().find(|p| p.id == id).cloned())
        .or_else(|| {
            state
                .data
                .plans
                .iter()
                .find(|p| p.term_code == term)
                .cloned()
        });
    let courses = plan
        .as_ref()
        .map(|p| {
            p.courses
                .iter()
                .map(|c| (c.key.to_string(), c.is_required))
                .collect()
        })
        .unwrap_or_default();
    let prefs = plan
        .as_ref()
        .and_then(|p| p.preferences.clone())
        .unwrap_or_default();
    let generated = state
        .generated
        .iter()
        .take(12)
        .map(|g| SchedLine {
            crns: g.crns.join(", "),
            reasons: g
                .reasons
                .iter()
                .map(|r| r.text.clone())
                .collect::<Vec<_>>()
                .join(" · "),
        })
        .collect();
    let picked = state.picked.min(state.generated.len().saturating_sub(1));
    let sections = state
        .generated
        .get(picked)
        .map(|g| g.sections.clone())
        .unwrap_or_default();
    let mut blocks = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        for meeting in section.weekly_meetings() {
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
                continue;
            };
            for day in meeting.days.indices() {
                blocks.push(WeekBlock {
                    day,
                    start,
                    end,
                    label: format!("{} {}", section.course_key(), section.crn),
                    color: i % 8,
                });
            }
        }
    }
    let suggestions =
        if state.generated.is_empty() && plan.as_ref().is_some_and(|p| !p.courses.is_empty()) {
            let inputs = plan_inputs(&state);
            let filters = plan.as_ref().map(|p| p.filters.clone()).unwrap_or_default();
            schedule::suggest_relaxations(&inputs, &filters, &prefs, true)
                .into_iter()
                .take(4)
                .map(|s| s.relaxation.title())
                .collect()
        } else {
            vec![]
        };
    ScheduleSnap {
        plans,
        plan,
        courses,
        pref_summary: prefs.summary_parts().join(" · "),
        generated,
        picked,
        total_label: state.generated.len().to_string(),
        suggestions,
        blocks,
        show_weekends: state.data.preferences.show_weekends,
        start_hour: state.data.preferences.calendar_start_hour,
        end_hour: state.data.preferences.calendar_end_hour,
    }
}

fn plan_inputs(state: &State) -> Vec<ScheduleCourseInput> {
    let Some(id) = current_plan_id(state) else {
        return vec![];
    };
    let Some(plan) = state.data.plans.iter().find(|p| p.id == id) else {
        return vec![];
    };
    schedule::inputs_for(plan, &state.index)
}

/// The degree plan on screen. The page shows the first plan when none has been
/// chosen yet, and Load requirements has to use that same plan.
fn active_degree_id(state: &State) -> Option<Uuid> {
    shown_degree_plan_id(state.degree, &state.data.degree_plans)
}

fn current_plan_id(state: &State) -> Option<Uuid> {
    let term = &state.index.catalog.term.code;
    state
        .plan
        .filter(|id| state.data.plans.iter().any(|p| p.id == *id))
        .or_else(|| {
            state
                .data
                .plans
                .iter()
                .find(|p| &p.term_code == term)
                .map(|p| p.id)
        })
}

fn degrees_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let snap = degree_snapshot(shell);
    if let Some(id) = snap.id {
        if shell.borrow().state.borrow().degree != Some(id) {
            shell.borrow().state.borrow_mut().degree = Some(id);
        }
    }
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    install_proportional_pane(&paned, page_pane_share(shell, 0.24));
    let side = gtk::Box::new(gtk::Orientation::Vertical, 6);
    side.set_margin_top(8);
    side.set_margin_start(8);
    let new_button = gtk::Button::with_label("New degree plan");
    new_button.connect_clicked({
        let shell = shell.clone();
        move |_| new_degree(&shell)
    });
    let import = gtk::Button::with_label("Import PDF");
    import.connect_clicked({
        let shell = shell.clone();
        move |_| import_degree(&shell)
    });
    side.append(&new_button);
    side.append(&import);
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    for (id, name) in &snap.plans {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        label.set_margin_start(8);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        let plan_id = *id;
        attach_context_menu(
            &row,
            vec![menu_action("Delete", {
                let shell = shell.clone();
                move || delete_degree_plan_ui(&shell, plan_id)
            })],
        );
        list.append(&row);
        if snap.id == Some(*id) {
            list.select_row(Some(&row));
        }
    }
    let ids: Vec<Uuid> = snap.plans.iter().map(|(id, _)| *id).collect();
    {
        let plan_ids: Rc<Vec<Uuid>> = Rc::new(ids.clone());
        let shell = shell.clone();
        attach_delete_key(
            &list,
            Rc::new(move |index| {
                if let Some(id) = plan_ids.get(index as usize) {
                    delete_degree_plan_ui(&shell, *id);
                }
            }),
        );
    }
    list.connect_row_selected({
        let shell = shell.clone();
        move |_, row| {
            let Some(row) = row else { return };
            if row.index() < 0 {
                return;
            }
            if let Some(id) = ids.get(row.index() as usize) {
                if shell.borrow().state.borrow().degree == Some(*id) {
                    return;
                }
                shell.borrow().state.borrow_mut().degree = Some(*id);
                refresh(&shell);
            }
        }
    });
    side.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build(),
    );
    paned.set_start_child(Some(&side));

    let main = gtk::Box::new(gtk::Orientation::Vertical, 12);
    main.set_margin_top(12);
    main.set_margin_bottom(18);
    main.set_margin_start(18);
    main.set_margin_end(18);
    if snap.id.is_none() {
        main.append(&dim_label(
            "Import a degree audit “View Plan” PDF, or start an empty plan and add semesters.",
        ));
        paned.set_end_child(Some(&main));
        return paned.upcast();
    }
    let tools = wrapping_box();
    for (label, which) in [
        ("Add semester", 0),
        ("Export PDF", 1),
        ("Autofill", 2),
        ("Load requirements", 3),
    ] {
        let button = gtk::Button::with_label(label);
        let shell = shell.clone();
        button.connect_clicked(move |_| match which {
            0 => add_semester(&shell),
            1 => export_pdf(&shell),
            2 => autofill_degree(&shell),
            _ => load_requirements(&shell),
        });
        tools.append(&button);
    }
    main.append(&tools);
    let plan_title = gtk::Label::new(Some(&snap.title));
    plan_title.add_css_class("title-1");
    plan_title.set_xalign(0.0);
    plan_title.set_wrap(true);
    main.append(&plan_title);
    main.append(&dim_label(&snap.totals));
    if !snap.checks.is_empty() {
        main.append(&heading_label("Checks"));
        for line in &snap.checks {
            main.append(&selectable(line));
        }
    }
    let catalog = adw::PreferencesGroup::new();
    catalog.set_title("Catalog");
    catalog.set_description(Some(
        "A path from the university catalog, used when you load requirements.",
    ));
    let path = adw::EntryRow::new();
    path.set_title("Path");
    path.set_text(&snap.catalog_path);
    path.connect_changed({
        let shell = shell.clone();
        move |row| shell.borrow().state.borrow_mut().catalog_path = row.text().to_string()
    });
    catalog.add(&path);
    main.append(&catalog);
    if !snap.audit.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.set_title("Requirements");
        for line in &snap.audit {
            let row = adw::ActionRow::new();
            row.set_title(line);
            row.set_title_lines(3);
            group.add(&row);
        }
        main.append(&group);
    }
    for listing in &snap.listings {
        main.append(&audit_listing_row(shell, listing));
    }
    main.append(&gap::equivalence_section(shell));
    main.append(&gap::petition_section(shell));
    main.append(&heading_label("Semesters"));
    main.append(&gap::semester_section(shell));
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let add = gtk::Entry::builder()
        .placeholder_text("Add a course, such as CS 3000")
        .hexpand(true)
        .build();
    let add_crn = gtk::Entry::builder()
        .placeholder_text("CRN")
        .width_chars(8)
        .build();
    let add_button = gtk::Button::with_label("Add");
    let submit_add = {
        let shell = shell.clone();
        let add = add.clone();
        let add_crn = add_crn.clone();
        move || {
            if add_degree_course(&shell, &add.text(), &add_crn.text()) {
                add.set_text("");
                add_crn.set_text("");
            }
        }
    };
    add.connect_activate({
        let submit_add = submit_add.clone();
        move |_| submit_add()
    });
    add_crn.connect_activate({
        let submit_add = submit_add.clone();
        move |_| submit_add()
    });
    add_button.connect_clicked(move |_| submit_add());
    add_row.append(&add);
    add_row.append(&add_crn);
    add_row.append(&add_button);
    main.append(&add_row);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(880);
    clamp.set_tightening_threshold(520);
    clamp.set_child(Some(&main));
    paned.set_end_child(Some(
        &gtk::ScrolledWindow::builder()
            .child(&clamp)
            .vexpand(true)
            .build(),
    ));
    paned.upcast()
}

struct AuditListing {
    key: CourseKey,
    on_plan: bool,
}

struct DegreeSnap {
    plans: Vec<(Uuid, String)>,
    id: Option<Uuid>,
    title: String,
    totals: String,
    checks: Vec<String>,
    audit: Vec<String>,
    listings: Vec<AuditListing>,
    catalog_path: String,
}

fn degree_snapshot(shell: &Rc<RefCell<Shell>>) -> DegreeSnap {
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let plans = state
        .data
        .degree_plans
        .iter()
        .map(|p| (p.id, p.name.clone()))
        .collect::<Vec<_>>();
    let id = active_degree_id(&state);
    let Some(plan) = id.and_then(|id| state.data.degree_plans.iter().find(|p| p.id == id)) else {
        return DegreeSnap {
            plans,
            id: None,
            title: String::new(),
            totals: String::new(),
            checks: vec![],
            audit: vec![],
            listings: vec![],
            catalog_path: state.catalog_path.clone(),
        };
    };
    let mut checks = Vec::new();
    let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(
        &CourseEquivalences::from_declarations(&plan.course_equivalences),
    );
    let dupes = planlogic::duplicate_keys(plan);
    if !dupes.is_empty() {
        checks.push(format!(
            "Listed more than once: {}",
            dupes
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (index, term) in plan.terms.iter().enumerate() {
        for course in &term.courses {
            let Some(key) = &course.key else { continue };
            let lookup = |key: &CourseKey| {
                state
                    .course_notes
                    .get(key)
                    .map(|info| info.prerequisites.clone())
            };
            let status = planlogic::prerequisite_status(
                key,
                index,
                plan,
                Some(&state.index),
                Some(&lookup),
                &equivalences,
            );
            if let PrereqStatus::Missing(text) = status {
                checks.push(format!(
                    "{} in {} is missing {text}",
                    key,
                    term.short_name()
                ));
            }
        }
    }
    let mut audit_lines = Vec::new();
    let mut listings: Vec<AuditListing> = Vec::new();
    if let Some(requirements) = state.requirements.get(&plan.id) {
        let audit = DegreeAudit::run(
            plan,
            requirements,
            &equivalences,
            &DegreeCourseAttributes::none(),
        );
        audit_lines.push(format!(
            "{}/{} requirements met · {:.0} completed hours, {:.0} planned",
            audit.satisfied_count(),
            audit.total_count(),
            audit.completed_hours,
            audit.planned_hours
        ));
        if let Some(hours) = audit.required_hours {
            audit_lines.push(format!("Catalog asks for {hours:.0} total semester hours"));
        }
        for rule in audit
            .rule_results()
            .into_iter()
            .filter(|r| r.status == AuditStatus::Missing || r.status == AuditStatus::Partial)
            .take(8)
        {
            audit_lines.push(format!("{:?} · {}", rule.status, rule.rule.label));
        }
        for note in audit.note_checks.iter().take(4) {
            audit_lines.push(format!("{:?} · {}", note.status, note.note));
        }
        for sub in audit.unmatched_substitutions.iter().take(4) {
            audit_lines.push(format!(
                "Petition not matched: {}",
                if sub.rule_title.is_empty() {
                    &sub.rule_id
                } else {
                    &sub.rule_title
                }
            ));
        }
        let on_plan: HashSet<CourseKey> = plan
            .completed
            .iter()
            .chain(plan.terms.iter().flat_map(|term| term.courses.iter()))
            .filter_map(|course| course.key.clone())
            .collect();
        let mut ranked = Vec::new();
        for rule in audit.rule_results() {
            let open = rule.status == AuditStatus::Missing || rule.status == AuditStatus::Partial;
            for key in rule.rule.listed_courses() {
                ranked.push((on_plan.contains(&key), open, key));
            }
        }
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.cmp(&left.1)));
        let mut seen = HashSet::new();
        for (on_plan, open, key) in ranked {
            if !seen.insert(key.clone()) {
                continue;
            }
            if !on_plan && !open {
                continue;
            }
            let repeats = listings.iter().filter(|item| item.on_plan).count();
            let fresh = listings.iter().filter(|item| !item.on_plan).count();
            if on_plan && repeats >= 12 {
                continue;
            }
            if !on_plan && fresh >= 6 {
                continue;
            }
            listings.push(AuditListing { key, on_plan });
        }
    }
    DegreeSnap {
        plans,
        id: Some(plan.id),
        title: plan.name.clone(),
        totals: format!(
            "{:.1} completed · {:.1} planned",
            plan.completed_credits(),
            plan.planned_credits()
        ),
        checks,
        audit: audit_lines,
        listings,
        catalog_path: state.catalog_path.clone(),
    }
}

fn watch_page(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let rows = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let now = now_secs();
        state
            .data
            .watchlist
            .iter()
            .map(|w| {
                let missing = state.missing_watch.contains(&w.id());
                (
                    w.crn.clone(),
                    w.course_key.clone(),
                    format!("{} · {}", w.course_key, w.course_title),
                    watchlist::row_line(w, missing, now),
                    w.notify_on_waitlist,
                )
            })
            .collect::<Vec<_>>()
    };
    let root = page_column("Watchlist", "Seat alerts run while Term Organize is open, and termorganize --seat-watch can run from a timer.");
    let check = gtk::Button::with_label("Check seats");
    check.add_css_class("suggested-action");
    check.set_halign(gtk::Align::Start);
    check.connect_clicked({
        let shell = shell.clone();
        move |_| check_seats(&shell)
    });
    root.append(&check);
    if rows.is_empty() {
        let status = adw::StatusPage::new();
        status.set_icon_name(Some("alarm-symbolic"));
        status.set_title("No sections watched");
        status.set_description(Some("Open a course and choose Watch on a section."));
        status.set_vexpand(true);
        root.append(&status);
        return root.upcast();
    }
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let watched: Rc<Vec<(String, CourseKey)>> = Rc::new(
        rows.iter()
            .map(|(crn, key, _, _, _)| (crn.clone(), key.clone()))
            .collect(),
    );
    for (crn, key, title, seats, wait) in rows {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(
            &gtk::Label::builder()
                .label(format!("CRN {crn} · {title}"))
                .xalign(0.0)
                .build(),
        );
        text.append(&dim_label(&seats));
        row.append(&text);
        let toggle = gtk::CheckButton::with_label("Waitlist");
        toggle.set_active(wait);
        let shell_t = shell.clone();
        let crn_t = crn.clone();
        toggle.connect_toggled(move |b| set_waitlist(&shell_t, &crn_t, b.is_active()));
        let remove = gtk::Button::with_label("Remove");
        let shell_r = shell.clone();
        let crn_r = crn.clone();
        remove.connect_clicked(move |_| remove_watch(&shell_r, &crn_r));
        row.append(&toggle);
        row.append(&remove);
        let item = gtk::ListBoxRow::new();
        item.set_child(Some(&row));
        attach_context_menu(
            &item,
            vec![
                menu_action("Open", {
                    let shell = shell.clone();
                    let key = key.clone();
                    move || gap::open_course_window(&shell, &key)
                }),
                menu_action("Copy CRN", {
                    let shell = shell.clone();
                    let crn = crn.clone();
                    move || copy_text(&shell, &crn, "CRN copied")
                }),
                menu_action("Remove", {
                    let shell = shell.clone();
                    move || remove_watch(&shell, &crn)
                }),
            ],
        );
        list.append(&item);
    }
    list.connect_row_activated({
        let shell = shell.clone();
        let watched = watched.clone();
        move |_, row| {
            if let Some((_, key)) = watched.get(row.index() as usize) {
                gap::open_course_window(&shell, key);
            }
        }
    });
    attach_delete_key(&list, {
        let shell = shell.clone();
        Rc::new(move |index| {
            if let Some((crn, _)) = watched.get(index as usize) {
                remove_watch(&shell, crn);
            }
        })
    });
    root.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build(),
    );
    root.upcast()
}

fn visible_days(blocks: &[WeekBlock], show_weekends: bool) -> Vec<i32> {
    let has = |day: i32| blocks.iter().any(|block| block.day == day);
    let mut days = vec![1, 2, 3, 4, 5];
    if show_weekends || has(6) {
        days.push(6);
    }
    if show_weekends || has(0) {
        days.insert(0, 0);
    }
    days
}

fn hour_span(blocks: &[WeekBlock], start_hour: i32, end_hour: i32) -> (i32, i32) {
    if blocks.is_empty() {
        return (start_hour, end_hour.max(start_hour + 1));
    }
    let min_start = blocks
        .iter()
        .map(|block| block.start)
        .min()
        .unwrap_or(start_hour * 60)
        / 60;
    let max_end_min = blocks
        .iter()
        .map(|block| block.end)
        .max()
        .unwrap_or(end_hour * 60);
    let max_end = (max_end_min + 59) / 60;
    let lower = start_hour.min(min_start).clamp(0, 23);
    let upper = end_hour.max(max_end).max(lower + 1).clamp(lower + 1, 24);
    (lower, upper)
}

struct WeekView {
    dark: bool,
    show_weekends: bool,
    start_hour: i32,
    end_hour: i32,
}

fn draw_week(
    cr: &gtk::cairo::Context,
    width: i32,
    height: i32,
    blocks: &[WeekBlock],
    view: &WeekView,
) {
    let width = width as f64;
    let height = height as f64;
    let WeekView {
        dark,
        show_weekends,
        start_hour,
        end_hour,
    } = *view;
    if dark {
        cr.set_source_rgb(0.12, 0.12, 0.14);
    } else {
        cr.set_source_rgb(0.97, 0.97, 0.96);
    }
    let _ = cr.paint();
    let days = visible_days(blocks, show_weekends);
    let (start_hour, end_hour) = hour_span(blocks, start_hour, end_hour);
    let start = start_hour * 60;
    let end = end_hour * 60;
    let header = 28.0;
    let left = 48.0;
    let col = (width - left - 8.0) / days.len().max(1) as f64;
    let span = (end - start).max(60) as f64;
    if dark {
        cr.set_source_rgb(0.75, 0.75, 0.78);
    } else {
        cr.set_source_rgb(0.25, 0.25, 0.28);
    }
    for hour in start_hour..=end_hour {
        let y = header + (hour * 60 - start) as f64 / span * (height - header - 8.0);
        cr.move_to(8.0, y);
        let _ = cr.show_text(&hour_axis_label(hour));
        cr.move_to(left, y);
        cr.line_to(width - 8.0, y);
    }
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    for (index, day) in days.iter().enumerate() {
        let x = left + index as f64 * col;
        cr.move_to(x + 6.0, 18.0);
        let _ = cr.show_text(DaySet::SHORT[*day as usize]);
    }
    let palette = [
        (0.20, 0.45, 0.75),
        (0.15, 0.55, 0.40),
        (0.75, 0.40, 0.20),
        (0.55, 0.30, 0.65),
        (0.70, 0.25, 0.35),
        (0.20, 0.55, 0.60),
        (0.55, 0.50, 0.15),
        (0.35, 0.35, 0.40),
    ];
    for block in blocks {
        let Some(index) = days.iter().position(|day| *day == block.day) else {
            continue;
        };
        let x = left + index as f64 * col + 3.0;
        let y = header + (block.start - start).max(0) as f64 / span * (height - header - 8.0);
        let h =
            ((block.end - block.start).max(20) as f64 / span * (height - header - 8.0)).max(16.0);
        let (r, g, b) = palette[block.color % palette.len()];
        cr.set_source_rgba(r, g, b, 0.9);
        cr.rectangle(x, y, col - 6.0, h);
        let _ = cr.fill();
        let _ = cr.save();
        cr.rectangle(x, y, (col - 6.0).max(1.0), h);
        cr.clip();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(x + 4.0, y + 14.0);
        let chars = ((col - 16.0) / 7.2).floor().max(1.0) as usize;
        let _ = cr.show_text(&short_text(&block.label, chars));
        let _ = cr.restore();
    }
}

fn draw_rooms(cr: &gtk::cairo::Context, width: i32, height: i32, view: &RoomCanvas) {
    let width = width as f64;
    let _height = height as f64;
    if view.dark {
        cr.set_source_rgb(0.12, 0.12, 0.14);
    } else {
        cr.set_source_rgb(0.98, 0.98, 0.97);
    }
    let _ = cr.paint();
    let inset = 14.0;
    let label_w = 228.0;
    let track_x = inset + label_w + 12.0;
    let track_w = (width - track_x - inset).max(48.0);
    let start = rooms::Moment::FIRST_CLASS;
    let end = rooms::Moment::LAST_CLASS;
    let span = f64::from(end - start);
    let x_at = |minutes: i32| {
        let clamped = minutes.clamp(start, end);
        track_x + track_w * f64::from(clamped - start) / span
    };
    if view.dark {
        cr.set_source_rgb(0.75, 0.75, 0.78);
    } else {
        cr.set_source_rgb(0.35, 0.35, 0.38);
    }
    cr.set_font_size(11.0);
    cr.move_to(inset, 18.0);
    let _ = cr.show_text("Room");
    let step = if track_w < 520.0 { 2 } else { 1 };
    let mut hour = start / 60;
    let last_hour = end / 60;
    while hour <= last_hour {
        cr.move_to(x_at(hour * 60) - 10.0, 18.0);
        let _ = cr.show_text(&hour_axis_label(hour));
        hour += step;
    }
    for (index, row) in view.rows.iter().enumerate() {
        let y = 30.0 + index as f64 * 36.0;
        let bar_y = y + 8.0;
        cr.set_font_size(13.0);
        if view.dark {
            cr.set_source_rgb(0.92, 0.92, 0.94);
        } else {
            cr.set_source_rgb(0.12, 0.12, 0.14);
        }
        cr.move_to(inset, y + 14.0);
        let _ = cr.show_text(&short_text(&row.label, 16));
        cr.set_font_size(11.0);
        if row.free {
            cr.set_source_rgb(0.16, 0.55, 0.32);
        } else if view.dark {
            cr.set_source_rgb(0.70, 0.70, 0.74);
        } else {
            cr.set_source_rgb(0.40, 0.40, 0.44);
        }
        cr.move_to(inset, y + 28.0);
        let _ = cr.show_text(&short_text(&row.status, 36));
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.16);
        cr.rectangle(track_x, bar_y, track_w, 18.0);
        let _ = cr.fill();
        for (from, to, past) in &row.free_spans {
            let x0 = x_at(*from);
            let alpha = if *past { 0.26 } else { 0.75 };
            cr.set_source_rgba(0.20, 0.62, 0.32, alpha);
            cr.rectangle(x0, bar_y, (x_at(*to) - x0).max(0.0), 18.0);
            let _ = cr.fill();
        }
        cr.set_source_rgba(1.0, 1.0, 1.0, if view.dark { 0.18 } else { 0.55 });
        cr.set_line_width(0.75);
        let mut line_hour = start / 60 + 1;
        while line_hour < last_hour {
            let x = x_at(line_hour * 60);
            cr.move_to(x, bar_y);
            cr.line_to(x, bar_y + 18.0);
            line_hour += 1;
        }
        let _ = cr.stroke();
        for (from, to, course) in &row.classes {
            let x0 = x_at(*from);
            let block_w = (x_at(*to) - x0).max(0.0);
            cr.set_source_rgba(0.45, 0.45, 0.48, 0.55);
            cr.rectangle(x0, bar_y, block_w, 18.0);
            let _ = cr.fill();
            if block_w >= 46.0 {
                if view.dark {
                    cr.set_source_rgba(0.95, 0.95, 0.96, 0.85);
                } else {
                    cr.set_source_rgba(0.10, 0.10, 0.12, 0.75);
                }
                cr.set_font_size(9.0);
                cr.move_to(x0 + 3.0, bar_y + 12.0);
                let chars = ((block_w - 6.0) / 6.0).floor().max(1.0) as usize;
                let _ = cr.show_text(&short_text(course, chars));
            }
        }
        if let Some(marker) = view.marker {
            if marker >= start && marker < end {
                let x = x_at(marker);
                cr.set_source_rgb(0.21, 0.48, 0.86);
                cr.rectangle(x - 1.0, bar_y - 4.0, 2.0, 26.0);
                let _ = cr.fill();
            }
        }
    }
}

fn short_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut trimmed: String = text.chars().take(max.saturating_sub(1)).collect();
    trimmed.push('…');
    trimmed
}

fn install_proportional_pane(paned: &gtk::Paned, default_share: f64) {
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(true);
    paned.set_shrink_end_child(true);
    paned.set_position((960.0 * default_share).round() as i32);
    paned.set_tooltip_text(Some("Drag the divider to resize. Drag it to the edge to hide the pane. Double-click it to restore the usual share."));
    let share = Rc::new(Cell::new(default_share));
    let collapsed = Rc::new(Cell::new(false));
    let applying = Rc::new(Cell::new(false));
    let paned_idle = paned.clone();
    let share_idle = share.clone();
    let applying_idle = applying.clone();
    paned.connect_realize(move |_| {
        let paned = paned_idle.clone();
        let share = share_idle.clone();
        let applying = applying_idle.clone();
        let tries = Rc::new(Cell::new(0i32));
        glib::idle_add_local(move || {
            tries.set(tries.get() + 1);
            let width = paned.width();
            if width < 200 {
                return if tries.get() < 8 {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                };
            }
            applying.set(true);
            paned.set_position((width as f64 * share.get()).round() as i32);
            applying.set(false);
            glib::ControlFlow::Break
        });
    });
    paned.connect_position_notify({
        let share = share.clone();
        let collapsed = collapsed.clone();
        let applying = applying.clone();
        move |paned| {
            if applying.get() {
                return;
            }
            let width = paned.width().max(1);
            let pos = paned.position();
            if collapsed.get() {
                if pos >= 80 {
                    collapsed.set(false);
                    share.set((pos as f64 / width as f64).clamp(0.12, 0.7));
                }
                return;
            }
            if pos >= 80 {
                let ratio = pos as f64 / width as f64;
                if ratio > 0.7 {
                    applying.set(true);
                    paned.set_position((width as f64 * 0.7).round() as i32);
                    applying.set(false);
                    share.set(0.7);
                } else {
                    share.set(ratio.max(0.12));
                }
            }
        }
    });
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let handle = paned.clone();
    click.connect_released(move |_, n_press, x, _| {
        let pos = handle.position();
        if n_press == 2 && (x as i32 - pos).abs() <= 18 {
            collapsed.set(false);
            share.set(default_share);
            let width = handle.width().max(1);
            applying.set(true);
            handle.set_position((width as f64 * default_share).round() as i32);
            applying.set(false);
            return;
        }
        if n_press == 1 && pos > 0 && pos < 80 {
            collapsed.set(true);
            applying.set(true);
            handle.set_position(0);
            applying.set(false);
        }
    });
    paned.add_controller(click);
}

type MenuAction = (String, Rc<dyn Fn()>);

fn menu_action(label: &str, action: impl Fn() + 'static) -> MenuAction {
    (label.to_string(), Rc::new(action))
}

fn attach_context_menu(widget: &impl IsA<gtk::Widget>, entries: Vec<MenuAction>) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let entries = Rc::new(entries);
    click.connect_pressed(move |gesture, _, x, y| {
        let _ = gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(widget) = gesture.widget() else {
            return;
        };
        show_context_menu(&widget, x, y, &entries);
    });
    widget.add_controller(click);
}

fn show_context_menu(widget: &impl IsA<gtk::Widget>, x: f64, y: f64, entries: &[MenuAction]) {
    if entries.is_empty() {
        return;
    }
    let popover = gtk::Popover::new();
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (label, action) in entries {
        let button = gtk::Button::with_label(label);
        button.add_css_class("flat");
        let action = Rc::clone(action);
        let popover = popover.clone();
        button.connect_clicked(move |_| {
            action();
            popover.popdown();
        });
        box_.append(&button);
    }
    popover.set_child(Some(&box_));
    popover.connect_closed(|popover| {
        popover.unparent();
    });
    popover.popup();
}

fn attach_delete_key(list: &gtk::ListBox, on_delete: Rc<dyn Fn(i32)>) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |controller, key, _, _| {
        if !matches!(
            key,
            gtk::gdk::Key::Delete | gtk::gdk::Key::KP_Delete | gtk::gdk::Key::BackSpace
        ) {
            return glib::Propagation::Proceed;
        }
        let Some(widget) = controller.widget() else {
            return glib::Propagation::Proceed;
        };
        let Ok(list) = widget.downcast::<gtk::ListBox>() else {
            return glib::Propagation::Proceed;
        };
        let Some(row) = list.selected_row() else {
            return glib::Propagation::Proceed;
        };
        if !row.is_selectable() {
            return glib::Propagation::Proceed;
        }
        on_delete(row.index());
        glib::Propagation::Stop
    });
    list.add_controller(keys);
}

fn copy_text(shell: &Rc<RefCell<Shell>>, text: &str, notice: &str) {
    shell.borrow().window.clipboard().set_text(text);
    toast(shell, notice);
}

fn column_headers(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_start(12);
    row.set_margin_top(4);
    for (label, sort) in [
        ("Code", SearchSort::CourseNumber),
        ("Title", SearchSort::Title),
        ("Open seats", SearchSort::OpenSeats),
    ] {
        let button = gtk::Button::with_label(label);
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Sort this column"));
        button.connect_clicked({
            let shell = shell.clone();
            move |_| {
                shell.borrow().state.borrow_mut().sort = sort;
                refresh(&shell);
            }
        });
        row.append(&button);
    }
    row.upcast()
}

fn attach_schedule_drop(widget: &impl IsA<gtk::Widget>, shell: &Rc<RefCell<Shell>>) {
    let target = gtk::DropTarget::new(
        glib::Type::STRING,
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    target.connect_drop({
        let shell = shell.clone();
        move |_, value, _, _| {
            let Ok(text) = value.get::<String>() else {
                return false;
            };
            if let Some(raw) = text.strip_prefix("key:") {
                let Some(key) = CourseKey::parse(raw) else {
                    return false;
                };
                add_course_to_plan(&shell, &key);
                return true;
            }
            if let Some(raw) = text.strip_prefix("id:") {
                let Some(id) = Uuid::parse_str(raw).ok() else {
                    return false;
                };
                let key = {
                    let shell_ref = shell.borrow();
                    let state = shell_ref.state.borrow();
                    state
                        .data
                        .degree_plans
                        .iter()
                        .flat_map(|plan| plan.terms.iter())
                        .flat_map(|term| term.courses.iter())
                        .find(|course| course.id == id)
                        .and_then(|course| course.key.clone())
                };
                let Some(key) = key else { return false };
                add_course_to_plan(&shell, &key);
                return true;
            }
            false
        }
    });
    widget.add_controller(target);
}

fn new_schedule(shell: &Rc<RefCell<Shell>>) {
    let id = {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state.index.catalog.term.code.clone();
        let plan = SchedulePlan {
            id: Uuid::new_v4(),
            name: format!(
                "Schedule {}",
                state
                    .data
                    .plans
                    .iter()
                    .filter(|p| p.term_code == term)
                    .count()
                    + 1
            ),
            term_code: term,
            courses: vec![],
            selected_crns: vec![],
            filters: ScheduleFilters::default(),
            preferences: None,
            favorite_combinations: vec![],
            is_primary: false,
            created_at: now_secs(),
            updated_at: now_secs(),
        };
        let id = state.data.add_schedule(plan);
        state.plan = Some(id);
        state.area = 5;
        state.generated.clear();
        persist(&mut state);
        id
    };
    let _ = id;
    select_area(shell, 5);
    toast(shell, "New schedule");
}

fn new_degree(shell: &Rc<RefCell<Shell>>) {
    let name = {
        let shell = shell.borrow();
        let state = shell.state.borrow();
        format!("Degree plan {}", state.data.degree_plans.len() + 1)
    };
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let id = state.data.add_degree_plan(DegreePlan {
            name,
            ..DegreePlan::default()
        });
        state.degree = Some(id);
        state.area = 6;
        persist(&mut state);
    }
    select_area(shell, 6);
}

fn add_course_to_plan(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    add_pinned_course(shell, key, None);
}

fn add_pinned_course(shell: &Rc<RefCell<Shell>>, key: &CourseKey, crn: Option<&str>) {
    let known = shell.borrow().state.borrow().index.course(key).is_some();
    if !known {
        toast(shell, "That course isn’t in this term");
        return;
    }
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state.index.catalog.term.code.clone();
        if current_plan_id(&state).is_none() {
            let plan = SchedulePlan {
                id: Uuid::new_v4(),
                name: "My schedule".into(),
                term_code: term,
                courses: vec![],
                selected_crns: vec![],
                filters: ScheduleFilters::default(),
                preferences: None,
                favorite_combinations: vec![],
                is_primary: true,
                created_at: now_secs(),
                updated_at: now_secs(),
            };
            let id = state.data.add_schedule(plan);
            state.plan = Some(id);
        }
        let id = current_plan_id(&state).unwrap();
        if let Some(plan) = state.data.plans.iter_mut().find(|p| p.id == id) {
            let existing = plan
                .courses
                .iter()
                .filter(|course| &course.key == key)
                .map(|course| course.pinned_crn.as_deref())
                .collect::<Vec<_>>();
            if planlogic::repeated_course_allowed(existing, crn) {
                let color = plan.next_color_index();
                let pin = planlogic::chosen_crn(crn).map(|text| text.to_string());
                plan.courses.push(PlanCourse {
                    key: key.clone(),
                    is_required: true,
                    excluded_crns: HashSet::new(),
                    color_index: color,
                    pinned_crn: pin.clone(),
                });
                if let Some(pin) = pin {
                    if !plan.selected_crns.iter().any(|item| item == &pin) {
                        plan.selected_crns.push(pin);
                    }
                }
                plan.updated_at = now_secs();
            }
        }
        state.generated.clear();
        persist(&mut state);
    }
    toast(shell, &format!("Added {key}"));
}

fn remove_plan_course(shell: &Rc<RefCell<Shell>>, key: &str) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(id) = current_plan_id(&state) else {
            return;
        };
        if let Some(plan) = state.data.plans.iter_mut().find(|p| p.id == id) {
            plan.courses.retain(|c| c.key.to_string() != key);
            plan.updated_at = now_secs();
        }
        state.generated.clear();
        persist(&mut state);
    }
    refresh(shell);
}

fn delete_schedule(shell: &Rc<RefCell<Shell>>, id: Uuid) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state
            .data
            .plans
            .iter()
            .find(|plan| plan.id == id)
            .map(|plan| plan.term_code.clone());
        let was_primary = state
            .data
            .plans
            .iter()
            .any(|plan| plan.id == id && plan.is_primary);
        state.data.plans.retain(|plan| plan.id != id);
        if was_primary {
            if let Some(term) = term {
                if let Some(plan) = state
                    .data
                    .plans
                    .iter_mut()
                    .find(|plan| plan.term_code == term)
                {
                    plan.is_primary = true;
                }
            }
        }
        if state.plan == Some(id) {
            state.plan = None;
        }
        state.generated.clear();
        persist(&mut state);
    }
    refresh(shell);
}

fn delete_degree_plan_ui(shell: &Rc<RefCell<Shell>>, id: Uuid) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        state.data.delete_degree_plan(id);
        if state.degree == Some(id) {
            state.degree = state.data.degree_plans.first().map(|plan| plan.id);
        }
        persist(&mut state);
    }
    refresh(shell);
}

fn remove_degree_course(shell: &Rc<RefCell<Shell>>, id: Uuid) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(plan_id) = state
            .degree
            .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
        else {
            return;
        };
        let Some(plan) = state
            .data
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == plan_id)
        else {
            return;
        };
        plan.completed.retain(|course| course.id != id);
        for term in &mut plan.terms {
            term.courses.retain(|course| course.id != id);
        }
        plan.updated_at = Some(now_secs().to_string());
        persist(&mut state);
    }
    refresh(shell);
}

fn unsave_course(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state.index.catalog.term.code.clone();
        state
            .data
            .saved
            .retain(|saved| !(saved.term_code == term && saved.key == *key));
        persist(&mut state);
    }
    refresh(shell);
}

fn generate(shell: &Rc<RefCell<Shell>>) {
    let outcome = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let inputs = plan_inputs(&state);
        let id = current_plan_id(&state);
        let plan = id.and_then(|id| state.data.plans.iter().find(|p| p.id == id));
        let filters = plan.map(|p| p.filters.clone()).unwrap_or_default();
        let prefs = plan.and_then(|p| p.preferences.clone()).unwrap_or_default();
        if inputs.is_empty() {
            None
        } else {
            let result = schedule::generate(
                &inputs,
                &filters,
                &prefs,
                schedule::ScheduleSort::Best,
                &schedule::Limits::default(),
            );
            Some((result.schedules, result.total_found, result.diagnostics))
        }
    };
    let Some((schedules, total, diagnostics)) = outcome else {
        toast(shell, "Add a course first");
        return;
    };
    if schedules.is_empty() {
        let title = schedule::no_results_title(&diagnostics);
        toast(shell, &title);
    } else {
        toast(shell, &format!("{total} conflict-free schedules"));
    }
    shell.borrow().state.borrow_mut().generated = schedules;
    shell.borrow().state.borrow_mut().picked = 0;
    shell.borrow().state.borrow_mut().area = 5;
    select_area(shell, 5);
}

fn apply_nl(shell: &Rc<RefCell<Shell>>, text: &str) {
    let message = match nl::interpret(text) {
        Ok(draft) => {
            let mut note = String::new();
            {
                let shell = shell.borrow();
                let mut state = shell.state.borrow_mut();
                if let Some(id) = current_plan_id(&state) {
                    if let Some(plan) = state.data.plans.iter_mut().find(|p| p.id == id) {
                        let base = plan.preferences.clone().unwrap_or_default();
                        let prefs = draft.applied(&base);
                        note = prefs.summary_parts().join(" · ");
                        plan.preferences = Some(prefs);
                        plan.updated_at = now_secs();
                        persist(&mut state);
                    }
                } else {
                    note = "Make a schedule first".into();
                }
                state.nl = text.to_string();
                state.generated.clear();
            }
            if note.is_empty() {
                "Preferences updated".into()
            } else {
                note
            }
        }
        Err(error) => error.message().to_string(),
    };
    toast(shell, &message);
    refresh(shell);
}

fn copy_crns(shell: &Rc<RefCell<Shell>>) {
    let text = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        state
            .generated
            .get(state.picked)
            .map(|g| schedule::copy_crns(&g.sections))
            .unwrap_or_default()
    };
    if text.is_empty() {
        toast(shell, "Generate a schedule first");
        return;
    }
    shell.borrow().window.clipboard().set_text(&text);
    toast(shell, "CRNs copied");
}

fn export_ics(shell: &Rc<RefCell<Shell>>) {
    let (body, name) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let Some(schedule) = state.generated.get(state.picked) else {
            drop(state);
            drop(shell_ref);
            toast(shell, "Generate a schedule first");
            return;
        };
        let id = current_plan_id(&state)
            .map(|id| id.to_string())
            .unwrap_or_else(|| "plan".into());
        let name = current_plan_id(&state)
            .and_then(|id| {
                state
                    .data
                    .plans
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
            })
            .unwrap_or_else(|| "Schedule".into());
        (schedule::ics(&id, &name, &schedule.sections), name)
    };
    save_text(shell, &format!("{name}.ics"), &body, "Calendar saved");
}

fn export_png(shell: &Rc<RefCell<Shell>>) {
    let bytes = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        state
            .generated
            .get(state.picked)
            .map(|schedule| schedule::week_png(&schedule.sections))
    };
    let Some(bytes) = bytes else {
        toast(shell, "Generate a schedule first");
        return;
    };
    save_bytes(shell, "week.png", &bytes, "Week image saved");
}

fn export_pdf(shell: &Rc<RefCell<Shell>>) {
    let (bytes, name) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let Some(id) = active_degree_id(&state) else {
            return;
        };
        let Some(plan) = state.data.degree_plans.iter().find(|p| p.id == id) else {
            return;
        };
        let include = state.data.preferences.include_student_in_export;
        (
            degree_format::pdf(plan, degree_format::now_unix(), include),
            format!("{}.pdf", plan.name),
        )
    };
    save_bytes(shell, &name, &bytes, "Degree plan PDF saved");
}

fn export_user(shell: &Rc<RefCell<Shell>>) {
    let text = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        store::export_json(
            &state.data,
            state.data.preferences.include_student_in_export,
        )
    };
    save_text(shell, "term-organize.json", &text, "Data exported");
}

fn add_semester(shell: &Rc<RefCell<Shell>>) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(id) = active_degree_id(&state) else {
            return;
        };
        if let Some(plan) = state.data.degree_plans.iter_mut().find(|p| p.id == id) {
            if let Some(next) = planlogic::next_semesters(plan, 1, 2026, 10)
                .into_iter()
                .next()
            {
                plan.terms.push(next);
                plan.sort_terms();
            }
        }
        persist(&mut state);
    }
    refresh(shell);
}

fn audit_listing_row(shell: &Rc<RefCell<Shell>>, listing: &AuditListing) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(&listing.key.to_string()));
    label.set_xalign(0.0);
    let crn = gtk::Entry::builder()
        .placeholder_text("CRN")
        .width_chars(8)
        .build();
    let button = gtk::Button::with_label(if listing.on_plan {
        "Add again"
    } else {
        "Add to plan"
    });
    let submit = {
        let shell = shell.clone();
        let crn = crn.clone();
        let code = listing.key.to_string();
        move || {
            if add_degree_course(&shell, &code, &crn.text()) {
                crn.set_text("");
            }
        }
    };
    crn.connect_activate({
        let submit = submit.clone();
        move |_| submit()
    });
    button.connect_clicked(move |_| submit());
    row.append(&label);
    row.append(&crn);
    row.append(&button);
    row.upcast()
}

fn add_degree_course(shell: &Rc<RefCell<Shell>>, text: &str, crn_text: &str) -> bool {
    let Some(key) = CourseKey::parse(text) else {
        toast(shell, "Use a course code like CS 3000");
        return false;
    };
    let outcome = {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let title = state
            .index
            .course(&key)
            .map(|course| course.title.clone())
            .unwrap_or_else(|| key.to_string());
        let credits = state
            .index
            .course(&key)
            .map(|course| course.max_credits)
            .unwrap_or(4.0);
        let Some(id) = active_degree_id(&state) else {
            return false;
        };
        state.degree = Some(id);
        let Some(plan) = state
            .data
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == id)
        else {
            return false;
        };
        if plan.terms.is_empty() {
            plan.terms.push(DegreePlanTerm::new(PlanSeason::Fall, 2026));
        }
        let crn = editing::clean_crn(Some(crn_text));
        let term_id =
            editing::term_with(&key, plan).unwrap_or_else(|| plan.terms.last().unwrap().id);
        let term_name = plan
            .terms
            .iter()
            .find(|term| term.id == term_id)
            .map(|term| term.name())
            .unwrap_or_default();
        if let Some(reason) = editing::block_reason(plan, &key, crn.as_deref(), None) {
            Err(reason)
        } else {
            let mut course = DegreePlanCourse::catalog(key.clone(), title, credits);
            course.crn = crn.clone();
            if editing::try_add(course, term_id, plan) {
                persist(&mut state);
                Ok((key, crn, term_name))
            } else {
                Err(format!("{key} could not be added."))
            }
        }
    };
    match outcome {
        Ok((key, crn, term_name)) => {
            let message = match crn {
                Some(crn) => format!("Added {key} (CRN {crn}) to {term_name}."),
                None => format!("Added {key} to {term_name}."),
            };
            toast(shell, &message);
            refresh(shell);
            true
        }
        Err(reason) => {
            toast(shell, &reason);
            false
        }
    }
}

fn save_degree_crn(shell: &Rc<RefCell<Shell>>, course_id: Uuid, text: &str) {
    let rejected = {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(plan_id) = state
            .degree
            .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
        else {
            return;
        };
        let Some(plan) = state
            .data
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == plan_id)
        else {
            return;
        };
        if editing::set_selected_crn(course_id, Some(text), plan) {
            persist(&mut state);
            None
        } else {
            let key = plan
                .completed
                .iter()
                .chain(plan.terms.iter().flat_map(|term| term.courses.iter()))
                .find(|course| course.id == course_id)
                .and_then(|course| course.key.clone());
            Some(
                key.and_then(|key| editing::block_reason(plan, &key, Some(text), Some(course_id)))
                    .unwrap_or_else(|| "That CRN could not be saved.".into()),
            )
        }
    };
    if let Some(reason) = rejected {
        toast(shell, &reason);
    }
    refresh(shell);
}

fn autofill_degree(shell: &Rc<RefCell<Shell>>) {
    let added = {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(id) = active_degree_id(&state) else {
            return;
        };
        state.degree = Some(id);
        let requirements = state.requirements.get(&id).cloned();
        let index_courses: HashSet<CourseKey> =
            state.index.courses.iter().map(|c| c.key()).collect();
        let year = state.index.catalog.term.calendar_year();
        let Some(plan) = state.data.degree_plans.iter_mut().find(|p| p.id == id) else {
            return;
        };
        let mut have: HashSet<CourseKey> = plan.completed_keys().into_iter().collect();
        for term in &plan.terms {
            have.extend(term.courses.iter().filter_map(|c| c.key.clone()));
        }
        let mut missing = Vec::new();
        if let Some(requirements) = requirements {
            let audit = DegreeAudit::run(
                plan,
                &requirements,
                &CourseEquivalences::none(),
                &DegreeCourseAttributes::none(),
            );
            for rule in audit.rule_results() {
                if rule.status.is_satisfied() {
                    continue;
                }
                for key in rule.rule.listed_courses() {
                    if !have.contains(&key) && index_courses.contains(&key) {
                        missing.push(key);
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        if plan.terms.is_empty() {
            plan.terms.push(DegreePlanTerm::new(PlanSeason::Fall, year));
        }
        let n = missing.len();
        let last = plan.terms.len() - 1;
        for key in missing {
            let title = key.to_string();
            plan.terms[last]
                .courses
                .push(DegreePlanCourse::catalog(key, title, 4.0));
        }
        persist(&mut state);
        n
    };
    toast(
        shell,
        &format!("Added {added} courses the audit still needs"),
    );
    refresh(shell);
}

fn autofill_schedule(shell: &Rc<RefCell<Shell>>) {
    let keys = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let term = state.index.catalog.term.clone();
        planlogic::schedule_suggestions(
            &term,
            &state.data.degree_plans,
            &state.data.plans,
            current_plan_id(&state),
            &HashSet::new(),
            Some(&state.index),
        )
        .into_iter()
        .next()
        .map(|suggestion| (suggestion.courses, suggestion.crns))
        .unwrap_or_default()
    };
    if keys.0.is_empty() {
        toast(shell, "No degree-plan courses to add for this term");
        return;
    }
    for (offset, key) in keys.0.iter().enumerate() {
        let crn = keys.1.get(offset).and_then(|crn| crn.as_deref());
        add_pinned_course(shell, key, crn);
    }
    refresh(shell);
}

fn load_requirements(shell: &Rc<RefCell<Shell>>) {
    let (path, id) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let id = active_degree_id(&state);
        (state.catalog_path.clone(), id)
    };
    let Some(id) = id else {
        toast(shell, "Open a degree plan first");
        return;
    };
    shell.borrow().state.borrow_mut().degree = Some(id);
    toast(shell, "Loading the catalog page…");
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
    std::thread::spawn(move || {
        let client = BannerClient {
            base: "https://catalog.northeastern.edu".into(),
        };
        let _ = tx.send(client.get(&path));
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
        Ok(Ok(html)) => {
            let year = CatalogYear::containing(2026, 10);
            let program = CatalogProgram {
                title: "Imported program".into(),
                path: shell.borrow().state.borrow().catalog_path.clone(),
            };
            match program_parser::requirements_from_html(&html, year, program) {
                Some(requirements) => {
                    shell
                        .borrow()
                        .state
                        .borrow_mut()
                        .requirements
                        .insert(id, requirements.clone());
                    let _ = save_requirements(id, &requirements);
                    toast(
                        &shell,
                        &format!("{} sections of requirements", requirements.sections.len()),
                    );
                    refresh(&shell);
                }
                None => toast(&shell, "That page didn’t contain a requirements table"),
            }
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            toast(
                &shell,
                &format!("Catalog unreachable ({error}). Open a saved HTML file instead."),
            );
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn import_degree(shell: &Rc<RefCell<Shell>>) {
    let dialog = gtk::FileDialog::builder()
        .title("Import a degree audit PDF")
        .accept_label("Import")
        .build();
    let filter = gtk::FileFilter::new();
    filter.add_mime_type("application/pdf");
    filter.add_pattern("*.pdf");
    filter.set_name(Some("Degree audit PDF"));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let shell = shell.clone();
    let window = shell.borrow().window.clone();
    dialog.open(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        match import_pdf(&path) {
            Ok(plan) => {
                let shell_ref = shell.borrow();
                let mut state = shell_ref.state.borrow_mut();
                let id = state.data.add_degree_plan(plan);
                state.degree = Some(id);
                state.area = 6;
                persist(&mut state);
                drop(state);
                drop(shell_ref);
                select_area(&shell, 6);
                toast(&shell, "Degree plan imported");
            }
            Err(error) => toast(&shell, &error),
        }
    });
}

fn import_pdf(path: &std::path::Path) -> Result<DegreePlan, String> {
    let output = std::process::Command::new("pdftotext")
        .args(["-layout", "-nopgbrk", path.to_str().unwrap_or(""), "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let name = path.file_name().and_then(|s| s.to_str());
    degree_format::parse(&text, name)
        .map(|r| r.plan)
        .map_err(|e| e.to_string())
}

fn save_course(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state.index.catalog.term.code.clone();
        if !state
            .data
            .saved
            .iter()
            .any(|s| s.term_code == term && s.key == *key)
        {
            state.data.saved.push(SavedCourse {
                term_code: term,
                key: key.clone(),
                saved_at: now_secs(),
            });
            persist(&mut state);
        }
    }
    toast(shell, "Saved");
}

fn remember_recent(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    let shell = shell.borrow();
    let mut state = shell.state.borrow_mut();
    let term = state.index.catalog.term.code.clone();
    state
        .data
        .recents
        .retain(|r| !(r.term_code == term && r.key == *key));
    state.data.recents.insert(
        0,
        RecentCourse {
            term_code: term,
            key: key.clone(),
            viewed_at: now_secs(),
        },
    );
    state.data.recents.truncate(40);
    persist(&mut state);
}

fn watch_section(shell: &Rc<RefCell<Shell>>, section: &CourseSection, title: &str) {
    let already = {
        let shell = shell.borrow();
        let state = shell.state.borrow();
        let term = &state.index.catalog.term.code;
        state
            .data
            .watchlist
            .iter()
            .any(|w| &w.term_code == term && w.crn == section.crn)
    };
    if already {
        toast(shell, "Already watched");
        return;
    }
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let term = state.index.catalog.term.code.clone();
        state.data.watchlist.push(WatchItem {
            term_code: term,
            crn: section.crn.clone(),
            course_key: section.course_key(),
            course_title: title.into(),
            section_label: section.section_number.clone(),
            added_at: now_secs(),
            last_seats_remaining: section.seats_remaining,
            last_waitlist_remaining: section.waitlist_remaining,
            seat_capacity: section.seat_capacity,
            last_checked_at: None,
            notify_on_waitlist: false,
            last_notified_at: None,
        });
        persist(&mut state);
    }
    toast(shell, "Watching that section");
}

fn set_waitlist(shell: &Rc<RefCell<Shell>>, crn: &str, on: bool) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        if let Some(item) = state.data.watchlist.iter_mut().find(|w| w.crn == crn) {
            item.notify_on_waitlist = on;
        }
        persist(&mut state);
    }
}

fn remove_watch(shell: &Rc<RefCell<Shell>>, crn: &str) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        state.data.watchlist.retain(|w| w.crn != crn);
        persist(&mut state);
    }
    refresh(shell);
}

fn apply_link(shell: &Rc<RefCell<Shell>>, link: AppLink) {
    match link {
        AppLink::Course {
            subject, number, ..
        } => {
            let key = CourseKey { subject, number };
            gap::open_course_window(shell, &key);
        }
        AppLink::Section { crn, .. } => {
            let key = shell
                .borrow()
                .state
                .borrow()
                .index
                .section_crn(&crn)
                .map(|s| s.course_key());
            if let Some(key) = key {
                gap::open_course_window(shell, &key);
            }
        }
        AppLink::Search { query } => {
            shell.borrow().state.borrow_mut().query = query;
            shell.borrow().state.borrow_mut().area = 0;
        }
        AppLink::Watchlist => shell.borrow().state.borrow_mut().area = 7,
        AppLink::Schedule { id } => {
            if let Some(id) = id.and_then(|s| Uuid::parse_str(&s).ok()) {
                shell.borrow().state.borrow_mut().plan = Some(id);
            }
            shell.borrow().state.borrow_mut().area = 5;
        }
        AppLink::Plan { id } => {
            if let Ok(id) = Uuid::parse_str(&id) {
                shell.borrow().state.borrow_mut().degree = Some(id);
            }
            shell.borrow().state.borrow_mut().area = 6;
        }
        AppLink::DegreePlans => shell.borrow().state.borrow_mut().area = 6,
        AppLink::File { path } => {
            if path.to_ascii_lowercase().ends_with(".pdf") {
                if let Ok(plan) = import_pdf(std::path::Path::new(&path)) {
                    let shell_ref = shell.borrow();
                    let mut state = shell_ref.state.borrow_mut();
                    let id = state.data.add_degree_plan(plan);
                    state.degree = Some(id);
                    state.area = 6;
                    persist(&mut state);
                }
            } else if let Ok(html) = std::fs::read_to_string(&path) {
                if let Some(id) = shell.borrow().state.borrow().degree {
                    if let Some(requirements) = program_parser::requirements_from_html(
                        &html,
                        CatalogYear::containing(2026, 10),
                        CatalogProgram {
                            title: "File".into(),
                            path,
                        },
                    ) {
                        shell
                            .borrow()
                            .state
                            .borrow_mut()
                            .requirements
                            .insert(id, requirements);
                    }
                }
            }
        }
    }
    let area = shell.borrow().state.borrow().area;
    select_area(shell, area);
}

fn open_settings(shell: &Rc<RefCell<Shell>>) {
    let window = adw::PreferencesWindow::new();
    window.set_transient_for(Some(&shell.borrow().window));
    window.set_modal(true);
    let page = adw::PreferencesPage::new();
    page.set_title("General");
    page.set_icon_name(Some("preferences-system-symbolic"));
    let appearance = adw::PreferencesGroup::new();
    appearance.set_title("Appearance");
    let scheme = adw::ComboRow::new();
    scheme.set_title("Color scheme");
    scheme.set_model(Some(&gtk::StringList::new(&[
        "Follow system",
        "Light",
        "Dark",
    ])));
    let current = shell
        .borrow()
        .state
        .borrow()
        .data
        .preferences
        .color_scheme
        .clone()
        .unwrap_or_else(|| "default".into());
    scheme.set_selected(match current.as_str() {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    });
    scheme.connect_selected_notify({
        let shell = shell.clone();
        move |row| {
            let value = match row.selected() {
                1 => Some("light".into()),
                2 => Some("dark".into()),
                _ => None,
            };
            {
                let shell = shell.borrow();
                let mut state = shell.state.borrow_mut();
                state.data.preferences.color_scheme = value;
                apply_scheme(&state.data.preferences);
                persist(&mut state);
            }
        }
    });
    appearance.add(&scheme);
    let weekends = adw::SwitchRow::new();
    weekends.set_title("Show weekends on the week view");
    weekends.set_active(shell.borrow().state.borrow().data.preferences.show_weekends);
    weekends.connect_active_notify({
        let shell = shell.clone();
        move |row| {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.show_weekends = row.is_active();
            persist(&mut state);
        }
    });
    appearance.add(&weekends);
    page.add(&appearance);

    let alerts = adw::PreferencesGroup::new();
    alerts.set_title("Notifications");
    alerts.set_description(Some(
        "Check seats asks Banner for the sections you are watching. The timer does the same when automatic checks are on.",
    ));
    let auto = adw::SwitchRow::new();
    auto.set_title("Check watched sections automatically");
    auto.set_subtitle("Uses the interval below while Term Organize is open");
    auto.set_active(
        shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .auto_refresh_seats,
    );
    auto.connect_active_notify({
        let shell = shell.clone();
        move |row| {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.auto_refresh_seats = row.is_active();
            persist(&mut state);
        }
    });
    alerts.add(&auto);
    let minutes = adw::SpinRow::with_range(5.0, 120.0, 5.0);
    minutes.set_title("Minutes between seat checks");
    minutes.set_value(
        shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .seat_check_minutes as f64,
    );
    minutes.connect_changed({
        let shell = shell.clone();
        move |row| {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.seat_check_minutes = row.value() as i32;
            persist(&mut state);
        }
    });
    alerts.add(&minutes);
    page.add(&alerts);

    let data_group = adw::PreferencesGroup::new();
    data_group.set_title("Data");
    data_group.set_description(Some("Exports leave out the student name and ID unless you include them. Folder sync is the stand-in for iCloud: a folder you choose, last file wins."));
    let include = adw::SwitchRow::new();
    include.set_title("Include student name and ID in exports");
    include.set_subtitle("They still stay off the sync folder");
    include.set_active(
        shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .include_student_in_export,
    );
    include.connect_active_notify({
        let shell = shell.clone();
        move |row| {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.include_student_in_export = row.is_active();
            persist(&mut state);
        }
    });
    data_group.add(&include);
    let sync_on = adw::SwitchRow::new();
    sync_on.set_title("Sync with a folder");
    sync_on.set_active(
        shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .folder_sync_enabled,
    );
    sync_on.connect_active_notify({
        let shell = shell.clone();
        move |row| {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.folder_sync_enabled = row.is_active();
            persist(&mut state);
        }
    });
    data_group.add(&sync_on);
    let path_row = adw::EntryRow::new();
    path_row.set_title("Folder");
    path_row.set_text(
        &shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .folder_sync_path
            .clone()
            .unwrap_or_default(),
    );
    path_row.connect_changed({
        let shell = shell.clone();
        move |row| {
            let text = row.text().trim().to_string();
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data.preferences.folder_sync_path =
                if text.is_empty() { None } else { Some(text) };
            persist(&mut state);
        }
    });
    data_group.add(&path_row);
    let export = adw::ActionRow::new();
    export.set_title("Export data");
    export.set_activatable(true);
    export.connect_activated({
        let shell = shell.clone();
        move |_| export_user(&shell)
    });
    data_group.add(&export);
    let import = adw::ActionRow::new();
    import.set_title("Import data");
    import.set_activatable(true);
    import.connect_activated({
        let shell = shell.clone();
        move |_| import_user(&shell)
    });
    data_group.add(&import);
    page.add(&data_group);

    let about = adw::PreferencesGroup::new();
    about.set_title("About");
    about.set_description(Some(legal::UNOFFICIAL_NOTICE));
    let license = adw::ActionRow::new();
    license.set_title(legal::LICENSE_NAME);
    license.set_subtitle(legal::SOURCE_CODE_URL);
    about.add(&license);
    let privacy = adw::ActionRow::new();
    privacy.set_title("Privacy");
    privacy.set_subtitle(legal::PRIVACY_NOTICE);
    about.add(&privacy);
    page.add(&about);
    window.add(&page);
    window.present();
}

fn open_about(shell: &Rc<RefCell<Shell>>) {
    let dialog = adw::AboutWindow::builder()
        .transient_for(&shell.borrow().window)
        .application_name(legal::APP_NAME)
        .application_icon("x-office-calendar")
        .version(env!("CARGO_PKG_VERSION"))
        .copyright(legal::APP_COPYRIGHT)
        .license_type(gtk::License::Agpl30Only)
        .website(legal::SOURCE_CODE_URL)
        .developers(vec!["Term Organize contributors"])
        .comments(format!(
            "{}\n\n{}\n\n{}",
            legal::UNOFFICIAL_NOTICE,
            legal::source_offer(),
            legal::SEARCHNEU_SUMMARY
        ))
        .build();
    dialog.present();
}

fn greet(shell: &Rc<RefCell<Shell>>) {
    let window = shell.borrow().window.clone();
    let dialog = adw::AlertDialog::new(Some("Unofficial"), Some(legal::UNOFFICIAL_NOTICE));
    dialog.set_body(&format!(
        "{}\n\n{}",
        legal::UNOFFICIAL_NOTICE,
        legal::DATA_NOTICE
    ));
    dialog.add_response("ok", "Continue");
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    let shell = shell.clone();
    dialog.connect_response(None, move |_, _| {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        state.data.preferences.has_completed_onboarding = true;
        persist(&mut state);
    });
    dialog.present(Some(&window));
}

fn import_user(shell: &Rc<RefCell<Shell>>) {
    let dialog = gtk::FileDialog::builder()
        .title("Import Term Organize data")
        .build();
    let shell = shell.clone();
    let window = shell.borrow().window.clone();
    dialog.open(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let imported = store::import_json(&text);
        {
            let shell = shell.borrow();
            let mut state = shell.state.borrow_mut();
            state.data = imported;
            persist(&mut state);
        }
        refresh(&shell);
        toast(&shell, "Data imported");
    });
}

fn arm_seat_timer(shell: &Rc<RefCell<Shell>>) {
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_secs(60), move || {
        let minutes = shell
            .borrow()
            .state
            .borrow()
            .data
            .preferences
            .seat_check_minutes
            .max(1) as u64;
        let due = {
            let shell_ref = shell.borrow();
            let state = shell_ref.state.borrow();
            if !state.data.preferences.auto_refresh_seats {
                return glib::ControlFlow::Continue;
            }
            state
                .data
                .watchlist
                .iter()
                .any(|w| w.last_checked_at.unwrap_or(0.0) + minutes as f64 * 60.0 < now_secs())
                || state
                    .data
                    .watchlist
                    .iter()
                    .any(|w| w.last_checked_at.is_none())
        };
        if due {
            check_seats(&shell);
        }
        glib::ControlFlow::Continue
    });
}

fn check_seats(shell: &Rc<RefCell<Shell>>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let (watched, snapshot, terms) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let watched = state.data.watchlist.clone();
        if watched.is_empty() {
            BUSY.store(false, Ordering::SeqCst);
            drop(state);
            drop(shell_ref);
            toast(shell, "Nothing to check");
            return;
        }
        let snapshot = state.index.catalog.sections.clone();
        let mut terms: Vec<String> = watched.iter().map(|item| item.term_code.clone()).collect();
        terms.sort();
        terms.dedup();
        (watched, snapshot, terms)
    };
    toast(shell, "Checking watched sections with Banner…");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let client = BannerClient::default();
        let mut live = HashMap::new();
        let mut error = None;
        for term in &terms {
            let items: Vec<_> = watched
                .iter()
                .filter(|item| &item.term_code == term)
                .cloned()
                .collect();
            let mut keys: Vec<(String, String)> = items
                .iter()
                .map(|item| {
                    (
                        item.course_key.subject.clone(),
                        item.course_key.number.clone(),
                    )
                })
                .collect();
            keys.sort();
            keys.dedup();
            let pairs: Vec<(String, CourseKey)> = items
                .iter()
                .map(|item| (item.crn.clone(), item.course_key.clone()))
                .collect();
            match client.live_seats_for_keys(term, &keys) {
                Ok(seats) => {
                    live.insert(
                        term.clone(),
                        banner::sections_with_live_seats(&snapshot, &pairs, &seats),
                    );
                }
                Err(message) => error = Some(message),
            }
        }
        let _ = tx.send((live, error));
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
        Ok((live, error)) => {
            BUSY.store(false, Ordering::SeqCst);
            if live.is_empty() {
                toast(
                    &shell,
                    &format!(
                        "Banner unreachable ({}). Showing the last check.",
                        error.unwrap_or_else(|| "no response".into())
                    ),
                );
                return glib::ControlFlow::Break;
            }
            let alerts = {
                let shell_ref = shell.borrow();
                let mut state = shell_ref.state.borrow_mut();
                let report = watchlist::compare(&state.data.watchlist, &live, now_secs());
                state.missing_watch = report.missing_ids.iter().cloned().collect();
                state.data.watchlist = report.items;
                persist(&mut state);
                report.alerts
            };
            if let Some(message) = error {
                toast(
                    &shell,
                    &format!("Some terms could not be checked ({message})."),
                );
            }
            for alert in alerts {
                let _ = std::process::Command::new("notify-send")
                    .args(["--app-name=Term Organize", &alert.title(), &alert.body()])
                    .status();
                toast(&shell, &alert.title());
            }
            refresh(&shell);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            BUSY.store(false, Ordering::SeqCst);
            glib::ControlFlow::Break
        }
    });
}

fn save_text(shell: &Rc<RefCell<Shell>>, name: &str, body: &str, message: &str) {
    let bytes = body.as_bytes().to_vec();
    save_bytes(shell, name, &bytes, message);
}

fn save_bytes(shell: &Rc<RefCell<Shell>>, name: &str, bytes: &[u8], message: &str) {
    let dialog = gtk::FileDialog::builder()
        .title("Save")
        .initial_name(name)
        .build();
    let shell = shell.clone();
    let window = shell.borrow().window.clone();
    let bytes = bytes.to_vec();
    let message = message.to_string();
    dialog.save(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        if std::fs::write(path, bytes).is_ok() {
            toast(&shell, &message);
        }
    });
}

fn persist(state: &mut State) {
    if let Ok((data, stamp)) = store::save_merging(&store::data_dir(), &state.data, &state.stamp) {
        state.data = data;
        state.stamp = stamp;
    }
    if state.data.preferences.folder_sync_enabled {
        if let Some(dir) = &state.data.preferences.folder_sync_path {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            let text = store::export_json(&state.data, false);
            let _ = std::fs::write(dir.join("user.json"), text);
        }
    }
}

/// Reloads `user.json` when another process writes it. Events are debounced so a
/// temp-file rename settles into one read.
fn watch_user_file(shell: &Rc<RefCell<Shell>>) {
    let path = store::user_path();
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(parent);
    let directory = gio::File::for_path(parent);
    let Ok(monitor) = directory.monitor_directory(
        gio::FileMonitorFlags::WATCH_MOVES,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };
    let generation = Rc::new(Cell::new(0u64));
    let shell_for_events = shell.clone();
    let generation_for_events = generation.clone();
    monitor.connect_changed(move |_, file, other, event| {
        if !matches!(
            event,
            gio::FileMonitorEvent::Changed
                | gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::Created
                | gio::FileMonitorEvent::Deleted
                | gio::FileMonitorEvent::Renamed
                | gio::FileMonitorEvent::Moved
                | gio::FileMonitorEvent::MovedIn
        ) {
            return;
        }
        let touches_user = basename_is_user_json(file) || other.is_some_and(basename_is_user_json);
        if !touches_user {
            return;
        }
        let next = generation_for_events.get().wrapping_add(1);
        generation_for_events.set(next);
        let generation = generation_for_events.clone();
        let shell = shell_for_events.clone();
        glib::timeout_add_local(Duration::from_millis(300), move || {
            if generation.get() != next {
                return glib::ControlFlow::Break;
            }
            apply_external_file(&shell);
            glib::ControlFlow::Break
        });
    });
    shell.borrow_mut().user_monitor = Some(monitor);
}

fn basename_is_user_json(file: &gio::File) -> bool {
    file.basename()
        .as_deref()
        .is_some_and(|name| name == std::ffi::OsStr::new("user.json"))
}

fn apply_external_file(shell: &Rc<RefCell<Shell>>) {
    {
        let shell_ref = shell.borrow();
        shell_ref
            .reload_pane_share
            .set(content_pane_share(&shell_ref.content));
    }
    let (data, stamp) = {
        let shell = shell.borrow();
        let state = shell.state.borrow();
        (state.data.clone(), state.stamp.clone())
    };
    let Ok(Some((data, stamp))) = store::fold_external(&store::data_dir(), &data, &stamp) else {
        return;
    };
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        state.data = data;
        state.stamp = stamp;
    }
    refresh(shell);
}

fn pull_folder(data: &mut UserData) {
    if !data.preferences.folder_sync_enabled {
        return;
    }
    let Some(dir) = data.preferences.folder_sync_path.clone() else {
        return;
    };
    let remote = PathBuf::from(&dir).join("user.json");
    let local = store::user_path();
    let remote_time = std::fs::metadata(&remote).and_then(|m| m.modified()).ok();
    let local_time = std::fs::metadata(&local).and_then(|m| m.modified()).ok();
    if remote_time > local_time {
        if let Ok(text) = std::fs::read_to_string(remote) {
            let imported = store::import_json(&text);
            let scheme = data.preferences.color_scheme.clone();
            let minutes = data.preferences.seat_check_minutes;
            *data = imported;
            data.preferences.color_scheme = scheme;
            data.preferences.seat_check_minutes = minutes;
            data.preferences.folder_sync_enabled = true;
            data.preferences.folder_sync_path = Some(dir);
        }
    }
}

fn apply_scheme(prefs: &AppPreferences) {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match prefs.color_scheme.as_deref() {
        Some("dark") => adw::ColorScheme::ForceDark,
        Some("light") => adw::ColorScheme::ForceLight,
        _ => adw::ColorScheme::Default,
    });
}

fn requirements_dir() -> PathBuf {
    store::data_dir().join("requirements")
}

fn save_requirements(id: Uuid, requirements: &ProgramRequirements) -> Result<(), String> {
    let dir = requirements_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(requirements).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{id}.json")), text).map_err(|e| e.to_string())
}

fn load_saved_requirements(state: &Rc<RefCell<State>>) {
    let dir = requirements_dir();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut state = state.borrow_mut();
    for entry in entries.flatten() {
        let Ok(id) = Uuid::parse_str(
            entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        ) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Ok(requirements) = serde_json::from_str::<ProgramRequirements>(&text) {
            state.requirements.insert(id, requirements);
        }
    }
}

fn sidebar_row(icon: &str, title: &str) -> gtk::ListBoxRow {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(10);
    row.set_margin_end(10);
    row.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    let item = gtk::ListBoxRow::new();
    item.set_child(Some(&row));
    item
}

fn page_column(title: &str, subtitle: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    root.append(&heading);
    root.append(&dim_label(subtitle));
    root
}

fn wrapping_box() -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_column_spacing(8);
    flow.set_row_spacing(8);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(8);
    flow.set_homogeneous(false);
    flow.set_hexpand(true);
    flow
}

fn heading_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

fn dim_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

fn meta_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_xalign(1.0);
    label.set_wrap(false);
    label
}

/// Break a long sidebar subtitle into lines so the pane cannot clip it mid-word.
fn soft_wrap(text: &str, width: usize) -> String {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let next = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if current.is_empty() || next <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}

fn selectable(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label
}

fn toast(shell: &Rc<RefCell<Shell>>, text: &str) {
    if text.is_empty() {
        return;
    }
    let toast = adw::Toast::new(text);
    shell.borrow().toast.add_toast(toast);
}

fn search_filter_row(shell: &Rc<RefCell<Shell>>, filters: &SearchFilters) -> gtk::Widget {
    let row = wrapping_box();
    row.set_max_children_per_line(5);
    let campuses = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let mut names = vec!["Any campus".to_string()];
        names.extend(state.index.campus_names.iter().take(12).cloned());
        names
    };
    let campus_refs: Vec<&str> = campuses.iter().map(|s| s.as_str()).collect();
    let campus = gtk::DropDown::from_strings(&campus_refs);
    let selected_campus = filters.campuses.iter().next().cloned();
    campus.set_selected(
        selected_campus
            .as_ref()
            .and_then(|name| campuses.iter().position(|c| c == name))
            .unwrap_or(0) as u32,
    );
    campus.connect_selected_notify({
        let shell = shell.clone();
        let campuses = campuses.clone();
        move |dd| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.campuses.clear();
            let index = dd.selected() as usize;
            if index > 0 {
                if let Some(name) = campuses.get(index) {
                    filters.campuses.insert(name.clone());
                }
            }
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let instruction = gtk::DropDown::from_strings(&["Any format", "In person", "Online"]);
    instruction.set_selected(match filters.instruction {
        search::InstructionFilter::Any => 0,
        search::InstructionFilter::InPerson => 1,
        search::InstructionFilter::Online => 2,
    });
    instruction.connect_selected_notify({
        let shell = shell.clone();
        move |dd| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.instruction = match dd.selected() {
                1 => search::InstructionFilter::InPerson,
                2 => search::InstructionFilter::Online,
                _ => search::InstructionFilter::Any,
            };
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let nupath = gtk::Entry::builder()
        .placeholder_text("NUpath, like WI")
        .text(
            filters
                .nupaths
                .iter()
                .map(|p| p.short())
                .collect::<Vec<_>>()
                .join(", "),
        )
        .width_chars(12)
        .build();
    nupath.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.nupaths.clear();
            for part in entry.text().split(|c: char| c == ',' || c.is_whitespace()) {
                if let Some(path) = NuPath::from_short(part) {
                    filters.nupaths.insert(path);
                }
            }
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let level = gtk::Entry::builder()
        .placeholder_text("Level")
        .text(
            filters
                .levels
                .iter()
                .next()
                .map(|n| n.to_string())
                .unwrap_or_default(),
        )
        .width_chars(6)
        .build();
    level.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.levels.clear();
            if let Ok(n) = entry.text().trim().parse::<i32>() {
                filters.levels.insert(n);
            }
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let credits = gtk::Entry::builder()
        .placeholder_text("Credits")
        .text(&match (filters.min_credits, filters.max_credits) {
            (Some(min), Some(max)) if min == max => format!("{min}"),
            (Some(min), None) => format!("{min}+"),
            _ => String::new(),
        })
        .width_chars(8)
        .build();
    credits.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            let text = entry.text().trim().to_string();
            filters.min_credits = None;
            filters.max_credits = None;
            if let Some(min) = text.strip_suffix('+').and_then(|s| s.parse().ok()) {
                filters.min_credits = Some(min);
            } else if let Ok(n) = text.parse::<f64>() {
                filters.min_credits = Some(n);
                filters.max_credits = Some(n);
            }
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let after = gtk::Entry::builder()
        .placeholder_text("After 10:00")
        .width_chars(10)
        .text(filters.starts_after.map(clock_label).unwrap_or_default())
        .build();
    after.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.starts_after = parse_clock(&entry.text());
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let before = gtk::Entry::builder()
        .placeholder_text("Before 17:00")
        .width_chars(10)
        .text(filters.ends_before.map(clock_label).unwrap_or_default())
        .build();
    before.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.ends_before = parse_clock(&entry.text());
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    let days = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    days.add_css_class("linked");
    for (day, bit) in [
        ("M", DaySet::MONDAY),
        ("T", DaySet::TUESDAY),
        ("W", DaySet::WEDNESDAY),
        ("R", DaySet::THURSDAY),
        ("F", DaySet::FRIDAY),
    ] {
        let button = gtk::ToggleButton::with_label(day);
        button.add_css_class("day-toggle");
        button.set_active(filters.allowed_days.is_some_and(|set| set.contains(bit)));
        button.connect_toggled({
            let shell = shell.clone();
            move |button| {
                let mut filters = shell.borrow().state.borrow().filters.clone();
                let mut set = filters.allowed_days.unwrap_or_default();
                if button.is_active() {
                    set.insert(bit);
                } else {
                    set.remove(bit);
                }
                filters.allowed_days = if set.is_empty() { None } else { Some(set) };
                shell.borrow().state.borrow_mut().filters = filters;
                refresh(&shell);
            }
        });
        days.append(&button);
    }
    row.append(&days);
    campus.set_hexpand(true);
    instruction.set_hexpand(true);
    nupath.set_hexpand(true);
    level.set_hexpand(true);
    credits.set_hexpand(true);
    after.set_hexpand(true);
    before.set_hexpand(true);
    let class_types = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let mut names = vec!["Any type".to_string()];
        names.extend(state.index.class_types.iter().take(16).cloned());
        names
    };
    let type_refs: Vec<&str> = class_types.iter().map(|name| name.as_str()).collect();
    let class_type = gtk::DropDown::from_strings(&type_refs);
    let selected_type = filters.class_types.iter().next().cloned();
    class_type.set_selected(
        selected_type
            .as_ref()
            .and_then(|name| class_types.iter().position(|item| item == name))
            .unwrap_or(0) as u32,
    );
    class_type.set_tooltip_text(Some("Class type"));
    class_type.set_hexpand(true);
    class_type.connect_selected_notify({
        let shell = shell.clone();
        let class_types = class_types.clone();
        move |menu| {
            let mut filters = shell.borrow().state.borrow().filters.clone();
            filters.class_types.clear();
            let index = menu.selected() as usize;
            if index > 0 {
                if let Some(name) = class_types.get(index) {
                    filters.class_types.insert(name.clone());
                }
            }
            shell.borrow().state.borrow_mut().filters = filters;
            refresh(&shell);
        }
    });
    row.append(&campus);
    row.append(&class_type);
    row.append(&instruction);
    row.append(&nupath);
    row.append(&level);
    row.append(&credits);
    row.append(&after);
    row.append(&before);
    row.upcast()
}

fn update_prefs(shell: &Rc<RefCell<Shell>>, change: impl FnOnce(&mut SchedulePreferences)) {
    {
        let shell = shell.borrow();
        let mut state = shell.state.borrow_mut();
        let Some(id) = current_plan_id(&state) else {
            return;
        };
        if let Some(plan) = state.data.plans.iter_mut().find(|p| p.id == id) {
            let mut prefs = plan.preferences.clone().unwrap_or_default();
            change(&mut prefs);
            plan.preferences = Some(prefs.normalized());
            plan.updated_at = now_secs();
            persist(&mut state);
        }
        state.generated.clear();
    }
    refresh(shell);
}

fn replace_course_window(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    let name = course_window_name(key);
    let windows = shell.borrow().windows.borrow().clone();
    for window in windows {
        if window.widget_name().as_str() != name || !window.is_visible() {
            continue;
        }
        let page = course_page(shell, key);
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(760);
        clamp.set_tightening_threshold(480);
        clamp.set_child(Some(&page));
        let scroll = gtk::ScrolledWindow::builder().child(&clamp).build();
        window.set_content(Some(&scroll));
    }
}

fn refresh_live_seats(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    let term = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        state.index.catalog.term.code.clone()
    };
    toast(shell, "Asking Banner for current seats…");
    let key = key.clone();
    let subject = key.subject.clone();
    let number = key.number.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(BannerClient::default().live_seats(&term, &subject, &number));
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
        Ok(Ok(seats)) => {
            let n = seats.len();
            {
                let shell = shell.borrow();
                let mut state = shell.state.borrow_mut();
                for seat in seats {
                    state.live_seats.insert(
                        seat.crn,
                        (
                            seat.seats_remaining,
                            seat.waitlist_remaining,
                            seat.seat_capacity,
                        ),
                    );
                }
            }
            toast(&shell, &format!("Updated {n} sections from Banner"));
            replace_course_window(&shell, &key);
            refresh(&shell);
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            toast(
                &shell,
                &format!("Banner unreachable ({error}). Showing the snapshot."),
            );
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn up_next_summary(state: &State) -> String {
    let Ok(now) = glib::DateTime::now_local() else {
        return String::new();
    };
    let glib_dow = now.day_of_week();
    let weekday = if glib_dow == 7 { 0 } else { glib_dow };
    let minutes = now.hour() * 60 + now.minute();
    let iso = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day_of_month()
    );
    let sections = if let Some(schedule) = state.generated.get(state.picked) {
        schedule.sections.clone()
    } else {
        let term = &state.index.catalog.term.code;
        let plan = state
            .plan
            .and_then(|id| state.data.plans.iter().find(|p| p.id == id))
            .or_else(|| state.data.plans.iter().find(|p| &p.term_code == term));
        let Some(plan) = plan else {
            return String::new();
        };
        plan.selected_crns
            .iter()
            .filter_map(|crn| state.index.section_crn(crn).cloned())
            .collect()
    };
    if sections.is_empty() {
        return String::new();
    }
    let items = termorganize_core::upnext::occurrences(&sections, weekday, minutes, &iso);
    let summary = termorganize_core::upnext::summary(&items);
    if summary == "No more classes today" {
        String::new()
    } else {
        format!("Up next: {summary}")
    }
}

fn install_drop(shell: &Rc<RefCell<Shell>>) {
    let target = gtk::DropTarget::new(gio::File::static_type(), gtk::gdk::DragAction::COPY);
    let dropped = shell.clone();
    target.connect_drop(move |_, value, _, _| {
        let Ok(file) = value.get::<gio::File>() else {
            return false;
        };
        let Some(path) = file.path() else {
            return false;
        };
        match import_pdf(&path) {
            Ok(plan) => {
                let shell_ref = dropped.borrow();
                let mut state = shell_ref.state.borrow_mut();
                let id = state.data.add_degree_plan(plan);
                state.degree = Some(id);
                state.area = 6;
                persist(&mut state);
                std::mem::drop(state);
                std::mem::drop(shell_ref);
                select_area(&dropped, 6);
                toast(&dropped, "Degree plan imported");
                true
            }
            Err(error) => {
                toast(&dropped, &error);
                false
            }
        }
    });
    shell.borrow().window.add_controller(target);
}

fn print_view(shell: &Rc<RefCell<Shell>>) {
    let text = view_text(shell);
    let lines = Rc::new(text.lines().map(|s| s.to_string()).collect::<Vec<_>>());
    let op = gtk::PrintOperation::new();
    op.set_job_name("Term Organize");
    op.set_unit(gtk::Unit::Points);
    let begin = lines.clone();
    op.connect_begin_print(move |op, ctx| {
        let per = ((ctx.height() - 72.0) / 14.0).floor().max(1.0) as i32;
        let per = per.max(1);
        let pages = ((begin.len() as i32) + per - 1) / per;
        op.set_n_pages(pages);
    });
    op.connect_draw_page(move |_, ctx, page| {
        let cr = ctx.cairo_context();
        let per = ((ctx.height() - 72.0) / 14.0).floor().max(1.0) as usize;
        let start = page as usize * per;
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.set_font_size(11.0);
        for (i, line) in lines.iter().skip(start).take(per).enumerate() {
            cr.move_to(36.0, 48.0 + i as f64 * 14.0);
            let _ = cr.show_text(line);
        }
    });
    let window = shell.borrow().window.clone();
    if let Err(error) = op.run(gtk::PrintOperationAction::PrintDialog, Some(&window)) {
        toast(shell, &error.to_string());
    }
}

fn view_text(shell: &Rc<RefCell<Shell>>) -> String {
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let mut lines = vec![
        format!("Term Organize — {}", state.index.catalog.term.name()),
        legal::UNOFFICIAL_NOTICE.to_string(),
    ];
    if let Some(key) = &state.course {
        if let Some(course) = state.index.course(key) {
            lines.push(format!("{} {}", course.code(), course.title));
            lines.push(course.description.clone());
        }
    }
    match state.area {
        5 => {
            if let Some(schedule) = state.generated.get(state.picked) {
                lines.push(schedule::copy_crns(&schedule.sections));
                for reason in &schedule.reasons {
                    lines.push(reason.text.clone());
                }
            }
        }
        6 => {
            if let Some(plan) = state
                .degree
                .and_then(|id| state.data.degree_plans.iter().find(|p| p.id == id))
            {
                lines.push(degree_format::text(
                    plan,
                    degree_format::now_unix(),
                    state.data.preferences.include_student_in_export,
                ));
            }
        }
        _ => {
            lines.push(state.query.clone());
        }
    }
    lines.join("\n")
}

fn request_background() {
    let Ok(proxy) = gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background",
        gio::Cancellable::NONE,
    ) else {
        return;
    };
    let exe = std::env::current_exe().ok();
    let Some(exe) = exe else { return };
    let command = [exe.to_string_lossy().to_string(), "--seat-watch".into()];
    let commandline =
        glib::Variant::array_from_iter::<glib::Variant>(command.iter().map(|s| s.to_variant()));
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", "termorganize");
    options.insert("reason", "Check watched sections for open seats");
    options.insert("autostart", true);
    options.insert("commandline", &commandline);
    let params = glib::Variant::tuple_from_iter(["".to_variant(), options.end()]);
    let _ = proxy.call_sync(
        "RequestBackground",
        Some(&params),
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    );
}

fn parse_clock(text: &str) -> Option<i32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some((h, m)) = text.split_once(':') {
        let hour: i32 = h.parse().ok()?;
        let minute: i32 = m.parse().ok()?;
        return Some(hour * 60 + minute);
    }
    let n: i32 = text.parse().ok()?;
    if n <= 24 {
        Some(n * 60)
    } else {
        Some(minutes_from_hhmm(n))
    }
}

fn clock_label(minutes: i32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod reload_layout {
    use super::{pane_share_after_reload, sidebar_after_reload};

    #[test]
    fn an_external_reload_keeps_the_sidebar_and_the_plan_list_width() {
        assert_eq!(sidebar_after_reload(true, true), (true, true));
        assert_eq!(sidebar_after_reload(false, false), (false, false));
        assert_eq!(pane_share_after_reload(Some(0.41), 0.24), 0.41);
        assert_eq!(pane_share_after_reload(Some(0.0), 0.24), 0.0);
        assert_eq!(pane_share_after_reload(None, 0.24), 0.24);
        assert_eq!(pane_share_after_reload(Some(0.95), 0.24), 0.7);
        assert_eq!(pane_share_after_reload(Some(f64::NAN), 0.24), 0.24);
    }
}

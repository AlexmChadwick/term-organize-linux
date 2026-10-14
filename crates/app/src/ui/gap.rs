//! Term menu, course windows, prerequisite trees, semester drag-and-drop,
//! petition questions, and the rest of the preferences survey.

use super::*;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::catalog;
use termorganize_core::course_info::{self, CourseInfoRepository};
use termorganize_core::editing;
use termorganize_core::offering::{OfferingHistory, OfferingRepository, OfferingSource};
use termorganize_core::outline::{
    GroupKind, ItemKind, PathContent, RequisiteItem, RequisiteLayout, RequisitePart,
};
use termorganize_core::petition::{self, CourseNumberChoice};
use termorganize_core::reorder;
use termorganize_core::suggestions;

struct LocalTerms {
    terms: Vec<Term>,
}

impl OfferingSource for LocalTerms {
    fn terms(&self) -> Result<Vec<Term>, String> {
        Ok(self.terms.clone())
    }
    fn offered_courses(
        &self,
        _term: &str,
        _courses: &[CourseKey],
    ) -> Result<HashSet<CourseKey>, String> {
        Err("offline".into())
    }
}

pub fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".req-chip { padding: 2px 4px; }
         .req-grey { opacity: 0.5; }
         .req-branch { margin-left: 12px; padding: 4px; }
         .semester-card { padding: 12px; }
         .day-toggle { min-width: 2.75em; padding: 6px 8px; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

pub fn term_label(term: &Term) -> String {
    let name = term.name();
    if name.is_empty() {
        term.code.clone()
    } else if name.contains(&term.code) {
        name
    } else {
        format!("{name} · {}", term.code)
    }
}

pub fn known_terms(current: &CatalogIndex) -> Vec<(String, String)> {
    let mut map = std::collections::BTreeMap::new();
    let mut add = |term: &Term| {
        map.entry(term.code.clone())
            .or_insert_with(|| term_label(term));
    };
    add(&current.catalog.term);
    if let Some(path) = bundled_catalog_path() {
        if let Ok(catalog) = archive::read(&path) {
            add(&catalog.term);
        }
    }
    for catalog in catalog::cached_catalogs() {
        add(&catalog.term);
    }
    let mut terms: Vec<_> = map.into_iter().collect();
    terms.sort_by(|a, b| b.0.cmp(&a.0));
    terms
}

pub fn install_term_menu(shell: &Rc<RefCell<Shell>>, smoke: bool) {
    let current = shell
        .borrow()
        .state
        .borrow()
        .index
        .catalog
        .term
        .code
        .clone();
    let terms = known_terms(&shell.borrow().state.borrow().index);
    apply_term_model(shell, &terms, &current);
    let menu = shell.borrow().term_menu.clone();
    menu.connect_selected_notify({
        let shell = shell.clone();
        move |menu| {
            if shell.borrow().guard.get() {
                return;
            }
            let index = menu.selected() as usize;
            let code = shell.borrow().state.borrow().term_codes.get(index).cloned();
            if let Some(code) = code {
                switch_term(&shell, &code);
            }
        }
    });
    if !smoke {
        refresh_remote_terms(shell);
    }
}

fn apply_term_model(shell: &Rc<RefCell<Shell>>, terms: &[(String, String)], selected: &str) {
    shell.borrow().guard.set(true);
    shell.borrow().state.borrow_mut().term_codes =
        terms.iter().map(|(code, _)| code.clone()).collect();
    shell.borrow().state.borrow_mut().term_labels =
        terms.iter().map(|(_, label)| label.clone()).collect();
    let labels: Vec<&str> = terms.iter().map(|(_, label)| label.as_str()).collect();
    let model = gtk::StringList::new(&labels);
    let menu = shell.borrow().term_menu.clone();
    menu.set_model(Some(&model));
    let index = terms
        .iter()
        .position(|(code, _)| code == selected)
        .unwrap_or(0) as u32;
    if !terms.is_empty() {
        menu.set_selected(index);
    }
    shell.borrow().guard.set(false);
}

fn refresh_remote_terms(shell: &Rc<RefCell<Shell>>) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(BannerClient::default().fetch_terms(120));
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(300), move || match rx.try_recv() {
        Ok(Ok(remote)) => {
            let current = shell
                .borrow()
                .state
                .borrow()
                .index
                .catalog
                .term
                .code
                .clone();
            let mut terms = known_terms(&shell.borrow().state.borrow().index);
            for term in remote {
                if !terms.iter().any(|(code, _)| code == &term.code) {
                    terms.push((term.code.clone(), term_label(&term)));
                }
            }
            terms.sort_by(|a, b| b.0.cmp(&a.0));
            apply_term_model(&shell, &terms, &current);
            glib::ControlFlow::Break
        }
        Ok(Err(_)) => glib::ControlFlow::Break,
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

pub fn index_for_code(code: &str) -> Option<CatalogIndex> {
    if let Some(mut catalog) = catalog::load_cached(code) {
        catalog.recompute_postreqs();
        return Some(CatalogIndex::build(catalog));
    }
    if let Some(path) = bundled_catalog_path() {
        if let Ok(mut catalog) = archive::read(&path) {
            if catalog.term.code == code {
                catalog.recompute_postreqs();
                return Some(CatalogIndex::build(catalog));
            }
        }
    }
    None
}

fn switch_term(shell: &Rc<RefCell<Shell>>, code: &str) {
    if shell.borrow().state.borrow().index.catalog.term.code == code {
        return;
    }
    if let Some(index) = index_for_code(code) {
        install_index(shell, index);
        return;
    }
    toast(
        shell,
        &format!("Downloading term {code} from Banner. This can take a minute."),
    );
    let code = code.to_string();
    let (knowledge, subjects, campuses, attributes, description) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let knowledge = state
            .index
            .catalog
            .courses
            .iter()
            .map(|course| (course.key(), course.clone()))
            .collect::<HashMap<_, _>>();
        let description = state
            .term_codes
            .iter()
            .position(|item| item == &code)
            .and_then(|index| state.term_labels.get(index))
            .map(|label| {
                label
                    .trim_end_matches(&format!(" · {code}"))
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        (
            knowledge,
            state.index.catalog.subjects.clone(),
            state.index.catalog.campuses.clone(),
            state.index.catalog.attributes.clone(),
            description,
        )
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let client = BannerClient::default();
        let result = (|| {
            let sections = client.fetch_all_sections(&code)?;
            let subjects = client.fetch_subjects(&code).ok().or(Some(subjects));
            let campuses = client.fetch_campuses(&code).ok().or(Some(campuses));
            let attributes = client.fetch_attributes(&code).ok().or(Some(attributes));
            let existing = catalog::load_cached(&code);
            let term = Term {
                code: code.clone(),
                banner_description: description,
            };
            let now = now_secs();
            let merged = catalog::merge(
                existing.as_ref(),
                &sections,
                term,
                subjects,
                campuses,
                attributes,
                &knowledge,
                now,
            );
            catalog::save_cached(&merged)?;
            Ok::<_, String>(merged.term.code)
        })();
        let _ = tx.send(result);
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(400), move || match rx.try_recv() {
        Ok(Ok(code)) => {
            if let Some(index) = index_for_code(&code) {
                toast(&shell, &format!("Loaded {}", index.catalog.term.name()));
                install_index(&shell, index);
            }
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            toast(&shell, &format!("Could not download that term ({error})."));
            let current = shell
                .borrow()
                .state
                .borrow()
                .index
                .catalog
                .term
                .code
                .clone();
            let terms = known_terms(&shell.borrow().state.borrow().index);
            apply_term_model(&shell, &terms, &current);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn install_index(shell: &Rc<RefCell<Shell>>, index: CatalogIndex) {
    let code = index.catalog.term.code.clone();
    {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
        state.data.preferences.selected_term_code = Some(code.clone());
        state.index = index;
        state.live_seats.clear();
        state.generated.clear();
        state.course = None;
        state.offering = OfferingHistory::empty();
        state.course_notes.clear();
        persist(&state.data);
    }
    let terms = known_terms(&shell.borrow().state.borrow().index);
    apply_term_model(shell, &terms, &code);
    refresh(shell);
}

pub fn open_course_window(shell: &Rc<RefCell<Shell>>, key: &CourseKey) {
    remember_recent(shell, key);
    let page = course_page(shell, key);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(760);
    clamp.set_tightening_threshold(480);
    clamp.set_child(Some(&page));
    let scroll = gtk::ScrolledWindow::builder().child(&clamp).build();
    let title = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        state
            .index
            .course(key)
            .map(|course| format!("{} · {}", course.code(), course.title))
            .unwrap_or_else(|| key.to_string())
    };
    let window = adw::Window::builder()
        .title(&title)
        .default_width(760)
        .default_height(680)
        .content(&scroll)
        .build();
    window.set_widget_name(&super::course_window_name(key));
    if let Some(app) = shell.borrow().window.application() {
        window.set_application(Some(&app));
    }
    window.present();
    shell.borrow().windows.borrow_mut().push(window);
}

pub fn open_professor_window(shell: &Rc<RefCell<Shell>>, name: &str) {
    shell.borrow().state.borrow_mut().professor = Some(name.to_string());
    let page = professor_page(shell);
    shell.borrow().state.borrow_mut().professor = None;
    let window = adw::Window::builder()
        .title(name)
        .default_width(720)
        .default_height(640)
        .content(&page)
        .build();
    if let Some(app) = shell.borrow().window.application() {
        window.set_application(Some(&app));
    }
    window.present();
    shell.borrow().windows.borrow_mut().push(window);
}

pub fn attach_course_drag(widget: &impl IsA<gtk::Widget>, key: &CourseKey) {
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::COPY);
    let payload = format!("key:{}", key);
    source.connect_prepare(move |_, _, _| {
        Some(gtk::gdk::ContentProvider::for_value(&payload.to_value()))
    });
    widget.add_controller(source);
}

pub fn requisite_block(
    shell: &Rc<RefCell<Shell>>,
    requisite: &Requisite,
    title: &str,
) -> gtk::Widget {
    let (offered, completed, equivalences) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let offered: HashSet<CourseKey> = state
            .index
            .courses
            .iter()
            .map(|course| course.key())
            .collect();
        let plan = state.data.primary_degree_plan();
        let completed: HashSet<CourseKey> = plan
            .map(|plan| plan.completed_keys().into_iter().collect())
            .unwrap_or_default();
        let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(
            &CourseEquivalences::from_declarations(
                plan.map(|plan| plan.course_equivalences.as_slice())
                    .unwrap_or(&[]),
            ),
        );
        (offered, completed, equivalences)
    };
    let Some(layout) = RequisiteLayout::build(
        requisite,
        |key| offered.contains(key),
        |key| {
            if completed.contains(key) {
                return (true, None);
            }
            match equivalences.satisfier(key, &completed) {
                Some(via) if via != key => (true, Some(via.clone())),
                Some(_) => (true, None),
                None => (false, None),
            }
        },
    ) else {
        return dim_label("None listed").upcast();
    };
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    box_.append(&draw_layout(shell, &layout));
    box_.append(&dim_label(&layout.spoken_description(title)));
    box_.upcast()
}

fn draw_layout(shell: &Rc<RefCell<Shell>>, layout: &RequisiteLayout) -> gtk::Widget {
    match layout {
        RequisiteLayout::Item(item) => chip(shell, item),
        RequisiteLayout::Group(group) => chip_row(shell, &group.items, group.kind),
        RequisiteLayout::Checklist(list) => {
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
            box_.append(&dim_label("All of these"));
            for part in &list.parts {
                box_.append(&draw_part(shell, part));
            }
            box_.upcast()
        }
        RequisiteLayout::Paths(paths) => {
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
            box_.append(&dim_label("One of these paths"));
            for (index, path) in paths.iter().enumerate() {
                if index > 0 {
                    box_.append(&dim_label("or"));
                }
                let branch = gtk::Box::new(gtk::Orientation::Vertical, 4);
                branch.add_css_class("card");
                branch.add_css_class("req-branch");
                branch.append(&draw_path(shell, &path.content));
                box_.append(&branch);
            }
            box_.upcast()
        }
    }
}

fn draw_path(shell: &Rc<RefCell<Shell>>, content: &PathContent) -> gtk::Widget {
    match content {
        PathContent::Item(item) => chip(shell, item),
        PathContent::Group(group) => chip_row(shell, &group.items, group.kind),
        PathContent::Checklist(list) => {
            draw_layout(shell, &RequisiteLayout::Checklist(list.clone()))
        }
    }
}

fn draw_part(shell: &Rc<RefCell<Shell>>, part: &RequisitePart) -> gtk::Widget {
    match part {
        RequisitePart::Item(item) => chip(shell, item),
        RequisitePart::Group(group) => chip_row(shell, &group.items, group.kind),
        RequisitePart::Expression(node) => draw_layout(shell, &RequisiteLayout::from_node(node)),
    }
}

fn chip_row(shell: &Rc<RefCell<Shell>>, items: &[RequisiteItem], kind: GroupKind) -> gtk::Widget {
    let row = gtk::FlowBox::new();
    row.set_selection_mode(gtk::SelectionMode::None);
    row.set_column_spacing(6);
    row.set_row_spacing(6);
    row.set_max_children_per_line(8);
    let word = match kind {
        GroupKind::All => "and",
        GroupKind::OneOf => "or",
    };
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            row.insert(&dim_label(word), -1);
        }
        row.insert(&chip(shell, item), -1);
    }
    row.upcast()
}

fn chip(shell: &Rc<RefCell<Shell>>, item: &RequisiteItem) -> gtk::Widget {
    let text = match &item.kind {
        ItemKind::Course(key) => {
            if let Some(via) = &item.via {
                format!("{key} via {via}")
            } else if item.is_complete {
                format!("{key} · done")
            } else {
                key.to_string()
            }
        }
        ItemKind::Test { name, .. } => match item.score_text() {
            Some(score) => format!("{name} · {score}"),
            None => name.clone(),
        },
    };
    let button = gtk::Button::with_label(&text);
    button.add_css_class("pill");
    button.add_css_class("req-chip");
    if item.is_complete {
        button.add_css_class("suggested-action");
    }
    if item.is_greyed_out() {
        button.add_css_class("req-grey");
    }
    if let Some(key) = item.course_key().cloned() {
        button.connect_clicked({
            let shell = shell.clone();
            move |_| open_course_window(&shell, &key)
        });
    }
    button.upcast()
}

pub fn semester_section(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let snap = semester_snapshot(shell);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let tools = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let older = gtk::Button::with_label("Check older terms");
    older.connect_clicked({
        let shell = shell.clone();
        move |_| refresh_older_terms(&shell)
    });
    tools.append(&older);
    tools.append(&dim_label("Drag a course onto another semester. Return opens it. Delete removes it. Older-term checks fill offering history and catalog details Banner still has."));
    root.append(&tools);
    if snap.is_empty() {
        root.append(&dim_label("Add a semester, then drop courses into it."));
        return root.upcast();
    }
    for semester in snap {
        root.append(&semester_card(shell, &semester));
    }
    root.upcast()
}

struct CourseRow {
    id: Uuid,
    key: Option<CourseKey>,
    label: String,
    detail: String,
}

struct SemesterSnap {
    id: Uuid,
    title: String,
    courses: Vec<CourseRow>,
    suggestions: Vec<reorder::Suggestion>,
}

fn semester_snapshot(shell: &Rc<RefCell<Shell>>) -> Vec<SemesterSnap> {
    ensure_local_offering(shell);
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let Some(plan_id) = state
        .degree
        .filter(|id| state.data.degree_plans.iter().any(|plan| plan.id == *id))
        .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
    else {
        return vec![];
    };
    let Some(plan) = state
        .data
        .degree_plans
        .iter()
        .find(|plan| plan.id == plan_id)
    else {
        return vec![];
    };
    let history = &state.offering;
    let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(
        &CourseEquivalences::from_declarations(&plan.course_equivalences),
    );
    let mut views = Vec::new();
    for term in &plan.terms {
        let mut courses = Vec::new();
        let mut blocking = Vec::new();
        for course in &term.courses {
            let detail = match &course.key {
                Some(key) => history.summary(key),
                None => String::new(),
            };
            if let Some(key) = &course.key {
                if history.is_likely_offered(key, term.season) == Some(false) {
                    blocking.push(key.clone());
                }
            }
            courses.push(CourseRow {
                id: course.id,
                key: course.key.clone(),
                label: format!("{} {}", course.label, course.title),
                detail,
            });
        }
        let notes = &state.course_notes;
        let index = &state.index;
        let suggestions = if blocking.is_empty() {
            vec![]
        } else {
            reorder::suggestions(
                plan,
                term.id,
                &blocking,
                history,
                |key| {
                    if let Some(course) = index.course(key) {
                        if course.details_loaded {
                            return Some(course.prereqs.clone());
                        }
                    }
                    notes.get(key).map(|info| info.prerequisites.clone())
                },
                &equivalences,
                20.0,
            )
        };
        views.push(SemesterSnap {
            id: term.id,
            title: format!("{} · {:.1} credits", term.name(), term.credits()),
            courses,
            suggestions,
        });
    }
    views
}

fn ensure_local_offering(shell: &Rc<RefCell<Shell>>) {
    let ready = !shell
        .borrow()
        .state
        .borrow()
        .offering
        .checked_terms
        .is_empty();
    if ready {
        return;
    }
    let shell_ref = shell.borrow();
    let mut state = shell_ref.state.borrow_mut();
    let keys = {
        let Some(plan) = state
            .degree
            .filter(|id| state.data.degree_plans.iter().any(|plan| plan.id == *id))
            .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
            .and_then(|id| state.data.degree_plans.iter().find(|plan| plan.id == id))
        else {
            return;
        };
        plan_keys(plan)
    };
    if keys.is_empty() {
        return;
    }
    state.offering = local_history(&state.index, &keys);
}

fn plan_keys(plan: &DegreePlan) -> Vec<CourseKey> {
    let mut keys = Vec::new();
    for course in plan
        .completed
        .iter()
        .chain(plan.terms.iter().flat_map(|term| term.courses.iter()))
    {
        if let Some(key) = &course.key {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn local_history(index: &CatalogIndex, keys: &[CourseKey]) -> OfferingHistory {
    let mut indexes = vec![index.clone()];
    for extra in catalog::indexes_on_disk() {
        if extra.catalog.term.code != index.catalog.term.code {
            indexes.push(extra);
        }
    }
    let terms = indexes
        .iter()
        .map(|index| index.catalog.term.clone())
        .collect();
    let mut repo = OfferingRepository::new(
        LocalTerms { terms },
        store::data_dir().join("offering-history.json"),
        now_secs(),
    );
    repo.history(keys, &indexes)
}

fn refresh_older_terms(shell: &Rc<RefCell<Shell>>) {
    let (index, keys) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let Some(plan) = state
            .degree
            .and_then(|id| state.data.degree_plans.iter().find(|plan| plan.id == id))
            .or_else(|| state.data.degree_plans.first())
        else {
            toast(shell, "Open a degree plan first.");
            return;
        };
        (state.index.clone(), plan_keys(plan))
    };
    if keys.is_empty() {
        toast(shell, "Add a course before checking older terms.");
        return;
    }
    toast(shell, "Asking Banner about older terms…");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut indexes = vec![index];
        for extra in catalog::indexes_on_disk() {
            if extra.catalog.term.code != indexes[0].catalog.term.code {
                indexes.push(extra);
            }
        }
        let now = now_secs();
        let mut offering_repo = OfferingRepository::new(
            BannerClient::default(),
            store::data_dir().join("offering-history.json"),
            now,
        );
        let history = offering_repo.history(&keys, &indexes);
        let missing: Vec<CourseKey> = keys
            .iter()
            .filter(|key| {
                indexes.iter().all(|index| {
                    index
                        .course(key)
                        .is_none_or(|course| !course.details_loaded)
                })
            })
            .cloned()
            .collect();
        let mut info_repo = CourseInfoRepository::new(
            BannerClient::default(),
            store::data_dir().join("course-info.json"),
            now,
        );
        let looked = info_repo.lookup(&missing, &indexes);
        let mut notes = HashMap::new();
        for (key, lookup) in looked {
            if let course_info::Lookup::Found(info) = lookup {
                notes.insert(key, info);
            }
        }
        let _ = tx.send((history, notes));
    });
    let shell = shell.clone();
    glib::timeout_add_local(Duration::from_millis(400), move || match rx.try_recv() {
        Ok((history, notes)) => {
            let checked = history.checked_terms.len();
            let found = notes.len();
            {
                let shell_ref = shell.borrow();
                let mut state = shell_ref.state.borrow_mut();
                state.offering = history;
                for (key, info) in notes {
                    state.course_notes.insert(key, info);
                }
            }
            toast(
                &shell,
                &format!("Checked {checked} terms. {found} older catalog entries."),
            );
            refresh(&shell);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

fn semester_card(shell: &Rc<RefCell<Shell>>, semester: &SemesterSnap) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.add_css_class("semester-card");
    card.append(&heading_label(&semester.title));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let count = semester.courses.len() as i32;
    let ids: Vec<Uuid> = semester.courses.iter().map(|course| course.id).collect();
    let keys: Vec<Option<CourseKey>> = semester
        .courses
        .iter()
        .map(|course| course.key.clone())
        .collect();
    for (index, course) in semester.courses.iter().enumerate() {
        list.append(&course_drag_row(shell, semester.id, course, index as i32));
    }
    list.connect_row_activated({
        let shell = shell.clone();
        let keys = keys.clone();
        move |_, row| {
            if let Some(Some(key)) = keys.get(row.index() as usize) {
                open_course_window(&shell, key);
            }
        }
    });
    super::attach_delete_key(&list, {
        let shell = shell.clone();
        let ids = ids.clone();
        std::rc::Rc::new(move |index| {
            if let Some(id) = ids.get(index as usize) {
                super::remove_degree_course(&shell, *id);
            }
        })
    });
    attach_semester_drop(&list, shell, semester.id, count);
    card.append(&list);
    for suggestion in &semester.suggestions {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.append(&selectable(&suggestion.title));
        row.append(&dim_label(&suggestion.reasons.join(" · ")));
        let apply = gtk::Button::with_label("Move");
        let suggestion = suggestion.clone();
        apply.connect_clicked({
            let shell = shell.clone();
            move |_| apply_move(&shell, &suggestion)
        });
        row.append(&apply);
        card.append(&row);
    }
    card.upcast()
}

fn course_drag_row(
    shell: &Rc<RefCell<Shell>>,
    term_id: Uuid,
    course: &CourseRow,
    position: i32,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    box_.set_margin_start(8);
    box_.set_margin_end(8);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let label = gtk::Label::new(Some(&course.label));
    label.set_xalign(0.0);
    label.set_wrap(true);
    text.append(&label);
    if !course.detail.is_empty() {
        text.append(&dim_label(&course.detail));
    }
    box_.append(&text);
    if let Some(key) = &course.key {
        let open = gtk::Button::with_label("Open");
        let key_open = key.clone();
        open.connect_clicked({
            let shell = shell.clone();
            move |_| open_course_window(&shell, &key_open)
        });
        box_.append(&open);
        attach_course_drag(&box_, key);
    }
    let mut entries = Vec::new();
    if let Some(key) = &course.key {
        let shell_open = shell.clone();
        let key_open = key.clone();
        entries.push(super::menu_action("Open", move || {
            open_course_window(&shell_open, &key_open)
        }));
        let shell_copy = shell.clone();
        let code = key.to_string();
        entries.push(super::menu_action("Copy code", move || {
            super::copy_text(&shell_copy, &code, "Course code copied")
        }));
    }
    let shell_remove = shell.clone();
    let course_id = course.id;
    entries.push(super::menu_action("Remove", move || {
        super::remove_degree_course(&shell_remove, course_id)
    }));
    super::attach_context_menu(&row, entries);
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::MOVE);
    let payload = format!("id:{}", course.id);
    source.connect_prepare(move |_, _, _| {
        Some(gtk::gdk::ContentProvider::for_value(&payload.to_value()))
    });
    row.add_controller(source);
    row.set_child(Some(&box_));
    attach_semester_drop(&row, shell, term_id, position);
    row
}

fn attach_semester_drop(
    widget: &impl IsA<gtk::Widget>,
    shell: &Rc<RefCell<Shell>>,
    term_id: Uuid,
    position: i32,
) {
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
            apply_drop(&shell, &text, term_id, position)
        }
    });
    widget.add_controller(target);
}

fn apply_drop(shell: &Rc<RefCell<Shell>>, payload: &str, term_id: Uuid, position: i32) -> bool {
    let added = if let Some(raw) = payload.strip_prefix("key:") {
        let Some(key) = CourseKey::parse(raw) else {
            return false;
        };
        let (title, credits) = {
            let shell_ref = shell.borrow();
            let state = shell_ref.state.borrow();
            state
                .index
                .course(&key)
                .map(|course| (course.title.clone(), course.max_credits))
                .unwrap_or_else(|| (key.to_string(), 4.0))
        };
        Some(DegreePlanCourse::catalog(key, title, credits))
    } else {
        None
    };
    let moved = {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
        let Some(plan_id) = state
            .degree
            .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
        else {
            return false;
        };
        let Some(plan) = state
            .data
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == plan_id)
        else {
            return false;
        };
        if let Some(id) = payload
            .strip_prefix("id:")
            .and_then(|raw| Uuid::parse_str(raw).ok())
        {
            editing::move_course_at(id, term_id, position, plan);
        } else if let Some(course) = added {
            let id = course.id;
            editing::add(course, term_id, plan);
            editing::move_course_at(id, term_id, position, plan);
        } else {
            return false;
        }
        plan.updated_at = Some(now_secs().to_string());
        persist(&state.data);
        true
    };
    if moved {
        refresh(shell);
    }
    moved
}

fn apply_move(shell: &Rc<RefCell<Shell>>, suggestion: &reorder::Suggestion) {
    {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
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
        reorder::apply(suggestion, plan);
        plan.updated_at = Some(now_secs().to_string());
        persist(&state.data);
    }
    toast(shell, &suggestion.title);
    refresh(shell);
}

pub fn petition_section(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let (questions, misses) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let Some(plan) = state
            .degree
            .and_then(|id| state.data.degree_plans.iter().find(|plan| plan.id == id))
            .or_else(|| state.data.degree_plans.first())
        else {
            return root.upcast();
        };
        let Some(requirements) = state.requirements.get(&plan.id) else {
            return root.upcast();
        };
        let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(
            &CourseEquivalences::from_declarations(&plan.course_equivalences),
        );
        let questions = petition::questions(plan, requirements, &equivalences, None);
        let audit = DegreeAudit::run(
            plan,
            requirements,
            &equivalences,
            &DegreeCourseAttributes::none(),
        );
        let mut misses = Vec::new();
        for result in audit.rule_results() {
            for miss in audit.near_misses(&result.rule.id) {
                misses.push(format!(
                    "{} · {} · {}",
                    result.rule.display_label(),
                    miss.course.key,
                    miss.reason
                ));
            }
        }
        (questions, misses)
    };
    if questions.is_empty() && misses.is_empty() {
        return root.upcast();
    }
    if !questions.is_empty() {
        root.append(&heading_label("Petition questions"));
        root.append(&dim_label("A petition copied a requirement that has a course-number minimum. Say whether that minimum still applies."));
        for question in questions {
            root.append(&petition_card(shell, &question));
        }
    }
    if !misses.is_empty() {
        root.append(&heading_label("Near misses"));
        for line in misses.iter().take(12) {
            root.append(&selectable(line));
        }
    }
    root.upcast()
}

fn petition_card(
    shell: &Rc<RefCell<Shell>>,
    question: &petition::DegreePetitionQuestion,
) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.add_css_class("semester-card");
    card.append(&heading_label(&question.question()));
    card.append(&selectable(&question.explanation()));
    let minimum = CourseNumberChoice::suggested_minimum(&question.ranges, None);
    let entry = gtk::Entry::builder()
        .placeholder_text("Minimum, like 2000")
        .text(minimum.to_string())
        .build();
    let group = gtk::CheckButton::with_label(
        &CourseNumberChoice::SameAsRequirement.title(&question.like_title),
    );
    group.set_tooltip_text(Some(
        &CourseNumberChoice::SameAsRequirement.detail(&question.ranges, minimum),
    ));
    let any =
        gtk::CheckButton::with_label(&CourseNumberChoice::AnyNumber.title(&question.like_title));
    any.set_group(Some(&group));
    any.set_tooltip_text(Some(
        &CourseNumberChoice::AnyNumber.detail(&question.ranges, minimum),
    ));
    let least =
        gtk::CheckButton::with_label(&CourseNumberChoice::AtLeast.title(&question.like_title));
    least.set_group(Some(&group));
    least.set_tooltip_text(Some(
        &CourseNumberChoice::AtLeast.detail(&question.ranges, minimum),
    ));
    let id = question.id();
    let wire = |button: &gtk::CheckButton, choice: CourseNumberChoice, entry: &gtk::Entry| {
        button.connect_toggled({
            let shell = shell.clone();
            let entry = entry.clone();
            move |button| {
                if !button.is_active() {
                    return;
                }
                let minimum = entry.text().parse().unwrap_or(minimum);
                answer_petition(&shell, id, choice, minimum);
            }
        });
    };
    wire(&group, CourseNumberChoice::SameAsRequirement, &entry);
    wire(&any, CourseNumberChoice::AnyNumber, &entry);
    wire(&least, CourseNumberChoice::AtLeast, &entry);
    entry.connect_activate({
        let shell = shell.clone();
        let least = least.clone();
        move |entry| {
            if !least.is_active() {
                return;
            }
            let minimum = entry.text().parse().unwrap_or(0);
            answer_petition(&shell, id, CourseNumberChoice::AtLeast, minimum);
        }
    });
    card.append(&group);
    card.append(&dim_label(
        &CourseNumberChoice::SameAsRequirement.detail(&question.ranges, minimum),
    ));
    card.append(&any);
    card.append(&least);
    card.append(&entry);
    card.upcast()
}

fn answer_petition(shell: &Rc<RefCell<Shell>>, id: Uuid, choice: CourseNumberChoice, minimum: i32) {
    let limit = choice.limit(minimum);
    if choice == CourseNumberChoice::AtLeast && limit.is_none() {
        toast(shell, "Enter a course number from 1 to 9999.");
        return;
    }
    {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
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
        plan.set_course_numbers(limit, id);
        plan.updated_at = Some(now_secs().to_string());
        persist(&state.data);
    }
    refresh(shell);
}

pub fn preference_survey(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let prefs = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        current_plan_id(&state)
            .and_then(|id| state.data.plans.iter().find(|plan| plan.id == id))
            .and_then(|plan| plan.preferences.clone())
            .unwrap_or_default()
    };
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);

    let words = adw::PreferencesGroup::new();
    words.set_title("In your own words");
    words.set_description(Some("The description and the rows below feed the same generator. Required wishes drop sections. Preferred wishes only change the ranking."));
    let nl_text = shell.borrow().state.borrow().nl.clone();
    let nl = entry_row("Description", &nl_text);
    nl.connect_apply({
        let shell = shell.clone();
        move |row| apply_nl(&shell, &row.text())
    });
    let apply_row = adw::ActionRow::new();
    apply_row.set_title("Apply description");
    apply_row.set_subtitle("Or press Enter in the description");
    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    apply.set_valign(gtk::Align::Center);
    apply_row.add_suffix(&apply);
    apply.connect_clicked({
        let shell = shell.clone();
        let nl = nl.clone();
        move |_| apply_nl(&shell, &nl.text())
    });
    words.add(&nl);
    words.add(&apply_row);
    page.append(&words);

    let time = adw::PreferencesGroup::new();
    time.set_title("Time of day");
    time.set_description(Some("Required wishes drop sections. Preferred wishes only change the ranking. Press Enter in a text field to apply it."));
    for band in TimeBand::ALL {
        time.add(&band_row(shell, band, &prefs));
    }
    page.append(&time);

    let days = adw::PreferencesGroup::new();
    days.set_title("Days off");
    days.set_description(Some("Selected days have no classes."));
    let (picker, _) = day_picker(
        "Days",
        &[
            ("Mon", DaySet::MONDAY),
            ("Tue", DaySet::TUESDAY),
            ("Wed", DaySet::WEDNESDAY),
            ("Thu", DaySet::THURSDAY),
            ("Fri", DaySet::FRIDAY),
            ("Sat", DaySet::SATURDAY),
            ("Sun", DaySet::SUNDAY),
        ],
        |bit| prefs.days_off.contains(bit),
        {
            let shell = shell.clone();
            move |bit, on| {
                update_prefs(&shell, |prefs| {
                    if on {
                        prefs.days_off.insert(bit);
                    } else {
                        prefs.days_off.remove(bit);
                    }
                });
            }
        },
    );
    days.add(&picker);
    days.add(&switch_row(
        "Required",
        Some("Drop sections that meet on a day off"),
        prefs.days_off_strength == PreferenceStrength::Required,
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| prefs.days_off_strength = strength);
            }
        },
    ));
    page.append(&days);

    let limits = adw::PreferencesGroup::new();
    limits.set_title("Start and end");
    limits.set_description(Some("Leave a time blank to clear it."));
    let earliest = entry_row(
        "Nothing before",
        prefs
            .earliest_start
            .as_ref()
            .map(|limit| clock_label(limit.minutes))
            .unwrap_or_default(),
    );
    earliest.connect_apply({
        let shell = shell.clone();
        move |row| apply_clock(&shell, &row.text(), true)
    });
    limits.add(&earliest);
    limits.add(&switch_row(
        "Earliest start is required",
        None,
        prefs.earliest_start.as_ref().map(|limit| limit.strength)
            == Some(PreferenceStrength::Required),
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| {
                    if let Some(limit) = &mut prefs.earliest_start {
                        limit.strength = strength;
                    }
                });
            }
        },
    ));
    let latest = entry_row(
        "Nothing after",
        prefs
            .latest_end
            .as_ref()
            .map(|limit| clock_label(limit.minutes))
            .unwrap_or_default(),
    );
    latest.connect_apply({
        let shell = shell.clone();
        move |row| apply_clock(&shell, &row.text(), false)
    });
    limits.add(&latest);
    limits.add(&switch_row(
        "Latest end is required",
        None,
        prefs.latest_end.as_ref().map(|limit| limit.strength) == Some(PreferenceStrength::Required),
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| {
                    if let Some(limit) = &mut prefs.latest_end {
                        limit.strength = strength;
                    }
                });
            }
        },
    ));
    page.append(&limits);

    let shape = adw::PreferencesGroup::new();
    shape.set_title("Shape of the week");
    shape.add(&combo_row(
        "Day layout",
        None,
        &["No preference", "Fewer days", "Spread out"],
        match prefs.day_layout {
            DayLayout::NoPreference => 0,
            DayLayout::FewerDays => 1,
            DayLayout::SpreadOut => 2,
        },
        {
            let shell = shell.clone();
            move |index| {
                update_prefs(&shell, |prefs| {
                    prefs.day_layout = match index {
                        1 => DayLayout::FewerDays,
                        2 => DayLayout::SpreadOut,
                        _ => DayLayout::NoPreference,
                    };
                });
            }
        },
    ));
    shape.add(&combo_row(
        "Gaps",
        None,
        &["Compact", "No preference", "Gaps are fine"],
        match prefs.gaps {
            GapPreference::Compact => 0,
            GapPreference::NoPreference => 1,
            GapPreference::Fine => 2,
        },
        {
            let shell = shell.clone();
            move |index| {
                update_prefs(&shell, |prefs| {
                    prefs.gaps = match index {
                        0 => GapPreference::Compact,
                        2 => GapPreference::Fine,
                        _ => GapPreference::NoPreference,
                    };
                });
            }
        },
    ));
    shape.add(&combo_row(
        "Breaks",
        None,
        &[
            "Back-to-back is fine",
            "Avoid back-to-back",
            "At least a break",
        ],
        match prefs.breaks {
            BreakPreference::Fine => 0,
            BreakPreference::AvoidBackToBack => 1,
            BreakPreference::AtLeast => 2,
        },
        {
            let shell = shell.clone();
            move |index| {
                update_prefs(&shell, |prefs| {
                    prefs.breaks = match index {
                        1 => BreakPreference::AvoidBackToBack,
                        2 => BreakPreference::AtLeast,
                        _ => BreakPreference::Fine,
                    };
                });
            }
        },
    ));
    let max_classes = entry_row(
        "Max classes per day",
        prefs
            .max_classes_per_day
            .map(|n| n.to_string())
            .unwrap_or_default(),
    );
    max_classes.connect_apply({
        let shell = shell.clone();
        move |row| {
            let value = row.text().trim().parse::<i32>().ok();
            update_prefs(&shell, |prefs| prefs.max_classes_per_day = value);
        }
    });
    shape.add(&max_classes);
    let max_minutes = entry_row(
        "Max minutes per day",
        prefs
            .max_minutes_per_day
            .map(|n| n.to_string())
            .unwrap_or_default(),
    );
    max_minutes.connect_apply({
        let shell = shell.clone();
        move |row| {
            let value = row.text().trim().parse::<i32>().ok();
            update_prefs(&shell, |prefs| prefs.max_minutes_per_day = value);
        }
    });
    shape.add(&max_minutes);
    let minimum_break = entry_row(
        "Minimum break, minutes",
        prefs.minimum_break_minutes.to_string(),
    );
    minimum_break.connect_apply({
        let shell = shell.clone();
        move |row| {
            if let Ok(value) = row.text().trim().parse::<i32>() {
                update_prefs(&shell, |prefs| prefs.minimum_break_minutes = value);
            }
        }
    });
    shape.add(&minimum_break);
    shape.add(&switch_row(
        "Minimum break is required",
        None,
        prefs.minimum_break_strength == PreferenceStrength::Required,
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| prefs.minimum_break_strength = strength);
            }
        },
    ));
    shape.add(&switch_row(
        "Keep a lunch break",
        Some("Leave a gap around noon"),
        prefs.lunch.is_some(),
        {
            let shell = shell.clone();
            move |on| {
                update_prefs(&shell, |prefs| {
                    prefs.lunch = if on {
                        Some(LunchBreak::default())
                    } else {
                        None
                    }
                })
            }
        },
    ));
    page.append(&shape);

    let blocked = adw::PreferencesGroup::new();
    blocked.set_title("Blocked times");
    blocked.set_description(Some("A required block drops sections that overlap it."));
    for block in &prefs.blocked_times {
        blocked.add(&blocked_row(shell, block));
    }
    let label = entry_row("Label", "");
    let (block_days, day_buttons) = day_picker(
        "Days",
        &[
            ("Mon", DaySet::MONDAY),
            ("Tue", DaySet::TUESDAY),
            ("Wed", DaySet::WEDNESDAY),
            ("Thu", DaySet::THURSDAY),
            ("Fri", DaySet::FRIDAY),
        ],
        |_| false,
        |_, _| {},
    );
    let start = entry_row("Starts", "9:00");
    let end = entry_row("Ends", "12:00");
    let required = adw::SwitchRow::new();
    required.set_title("Required");
    required.set_subtitle("Drop overlapping sections");
    required.set_active(true);
    let add_row = adw::ActionRow::new();
    add_row.set_title("Add this block");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    add.set_valign(gtk::Align::Center);
    add_row.add_suffix(&add);
    blocked.add(&label);
    blocked.add(&block_days);
    blocked.add(&start);
    blocked.add(&end);
    blocked.add(&required);
    blocked.add(&add_row);
    add.connect_clicked({
        let shell = shell.clone();
        let day_buttons = day_buttons.clone();
        let required = required.clone();
        move |_| {
            let mut dayset = DaySet(0);
            for (button, bit) in day_buttons.iter() {
                if button.is_active() {
                    dayset.insert(*bit);
                }
            }
            let Some(start_m) = parse_clock(&start.text()) else {
                toast(&shell, "Enter a start time, like 9:00.");
                return;
            };
            let Some(end_m) = parse_clock(&end.text()) else {
                toast(&shell, "Enter an end time, like 12:00.");
                return;
            };
            let name = label.text().trim().to_string();
            let block = BlockedTime {
                id: Uuid::new_v4(),
                label: if name.is_empty() {
                    "Blocked time".into()
                } else {
                    name
                },
                days: dayset,
                start: start_m,
                end: end_m,
                strength: if required.is_active() {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                },
                is_enabled: true,
            };
            if !block.is_valid() {
                toast(
                    &shell,
                    "Pick at least one day, and make the end later than the start.",
                );
                return;
            }
            update_prefs(&shell, |prefs| prefs.blocked_times.push(block));
        }
    });
    page.append(&blocked);

    let format = adw::PreferencesGroup::new();
    format.set_title("Format, campus, and seats");
    format.add(&combo_row(
        "Format",
        None,
        &["No format preference", "In person", "Hybrid", "Online"],
        match prefs.format {
            FormatPreference::NoPreference => 0,
            FormatPreference::InPerson => 1,
            FormatPreference::Hybrid => 2,
            FormatPreference::Online => 3,
        },
        {
            let shell = shell.clone();
            move |index| {
                update_prefs(&shell, |prefs| {
                    prefs.format = match index {
                        1 => FormatPreference::InPerson,
                        2 => FormatPreference::Hybrid,
                        3 => FormatPreference::Online,
                        _ => FormatPreference::NoPreference,
                    };
                });
            }
        },
    ));
    format.add(&switch_row(
        "Format is required",
        None,
        prefs.format_strength == PreferenceStrength::Required,
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| prefs.format_strength = strength);
            }
        },
    ));
    let campus = entry_row("Campus", prefs.campus.clone().unwrap_or_default());
    campus.connect_apply({
        let shell = shell.clone();
        move |row| {
            let text = row.text().trim().to_string();
            update_prefs(&shell, |prefs| {
                prefs.campus = if text.is_empty() { None } else { Some(text) }
            });
        }
    });
    format.add(&campus);
    format.add(&switch_row(
        "Campus is required",
        None,
        prefs.campus_strength == PreferenceStrength::Required,
        {
            let shell = shell.clone();
            move |on| {
                let strength = if on {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
                update_prefs(&shell, |prefs| prefs.campus_strength = strength);
            }
        },
    ));
    format.add(&combo_row(
        "Seats",
        None,
        &["Any seats", "Waitlist is fine", "Open seats only"],
        match prefs.seats {
            SeatPolicy::Any => 0,
            SeatPolicy::WaitlistOk => 1,
            SeatPolicy::OpenOnly => 2,
        },
        {
            let shell = shell.clone();
            move |index| {
                update_prefs(&shell, |prefs| {
                    prefs.seats = match index {
                        1 => SeatPolicy::WaitlistOk,
                        2 => SeatPolicy::OpenOnly,
                        _ => SeatPolicy::Any,
                    };
                });
            }
        },
    ));
    page.append(&format);

    let people = adw::PreferencesGroup::new();
    people.set_title("Instructors and credits");
    let preferred = entry_row(
        "Preferred instructors",
        prefs.preferred_instructors.join(", "),
    );
    preferred.connect_apply({
        let shell = shell.clone();
        move |row| {
            let names = split_names(&row.text());
            update_prefs(&shell, |prefs| prefs.preferred_instructors = names);
        }
    });
    let avoided = entry_row("Avoided instructors", prefs.avoided_instructors.join(", "));
    avoided.connect_apply({
        let shell = shell.clone();
        move |row| {
            let names = split_names(&row.text());
            update_prefs(&shell, |prefs| prefs.avoided_instructors = names);
        }
    });
    people.add(&preferred);
    people.add(&avoided);
    let lower = entry_row(
        "Minimum credits",
        prefs
            .credit_target
            .as_ref()
            .map(|t| t.lower.to_string())
            .unwrap_or_default(),
    );
    let upper = entry_row(
        "Maximum credits",
        prefs
            .credit_target
            .as_ref()
            .map(|t| t.upper.to_string())
            .unwrap_or_default(),
    );
    let apply_credits = Rc::new({
        let shell = shell.clone();
        let lower = lower.clone();
        let upper = upper.clone();
        move || {
            let lo = lower.text().trim().parse::<i32>().ok();
            let hi = upper.text().trim().parse::<i32>().ok();
            update_prefs(&shell, |prefs| {
                prefs.credit_target = match (lo, hi) {
                    (Some(lo), Some(hi)) => Some(CreditTarget::new(lo, hi)),
                    (Some(lo), None) => Some(CreditTarget::new(lo, lo)),
                    (None, Some(hi)) => Some(CreditTarget::new(hi, hi)),
                    (None, None) => None,
                };
            });
        }
    });
    lower.connect_apply({
        let apply_credits = apply_credits.clone();
        move |_| apply_credits()
    });
    upper.connect_apply({
        let apply_credits = apply_credits.clone();
        move |_| apply_credits()
    });
    people.add(&lower);
    people.add(&upper);
    page.append(&people);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(720);
    clamp.set_tightening_threshold(480);
    clamp.set_margin_top(6);
    clamp.set_margin_bottom(12);
    clamp.set_child(Some(&page));
    clamp.upcast()
}

fn band_row(
    shell: &Rc<RefCell<Shell>>,
    band: TimeBand,
    prefs: &SchedulePreferences,
) -> adw::ComboRow {
    let selected = match prefs.band(band) {
        TimePreference::Avoid => 0,
        TimePreference::Okay => 1,
        TimePreference::Prefer => 2,
    };
    let shell = shell.clone();
    combo_row(
        band.title(),
        Some(band.detail()),
        &["Avoid", "Okay", "Prefer"],
        selected,
        move |index| {
            let pref = match index {
                0 => TimePreference::Avoid,
                2 => TimePreference::Prefer,
                _ => TimePreference::Okay,
            };
            update_prefs(&shell, |prefs| prefs.set_band(band, pref));
        },
    )
}

fn combo_row(
    title: &str,
    subtitle: Option<&str>,
    labels: &[&str],
    selected: u32,
    apply: impl Fn(u32) + 'static,
) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.set_model(Some(&gtk::StringList::new(labels)));
    row.set_selected(selected);
    row.connect_selected_notify(move |row| apply(row.selected()));
    row
}

fn switch_row(
    title: &str,
    subtitle: Option<&str>,
    active: bool,
    apply: impl Fn(bool) + 'static,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.set_active(active);
    row.connect_active_notify(move |row| apply(row.is_active()));
    row
}

fn entry_row(title: &str, text: impl AsRef<str>) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(title);
    row.set_text(text.as_ref());
    row
}

fn apply_clock(shell: &Rc<RefCell<Shell>>, text: &str, earliest: bool) {
    let minutes = parse_clock(text);
    update_prefs(shell, |prefs| {
        let limit = minutes.map(|minutes| TimeLimit {
            minutes,
            strength: PreferenceStrength::Required,
        });
        if earliest {
            if let (Some(next), Some(prev)) = (&limit, &prefs.earliest_start) {
                let mut next = next.clone();
                next.strength = prev.strength;
                prefs.earliest_start = Some(next);
            } else {
                prefs.earliest_start = limit;
            }
        } else if let (Some(next), Some(prev)) = (&limit, &prefs.latest_end) {
            let mut next = next.clone();
            next.strength = prev.strength;
            prefs.latest_end = Some(next);
        } else {
            prefs.latest_end = limit;
        }
    });
}

fn day_picker(
    title: &str,
    days: &[(&str, DaySet)],
    is_on: impl Fn(DaySet) -> bool,
    toggle: impl Fn(DaySet, bool) + 'static,
) -> (adw::PreferencesRow, Rc<Vec<(gtk::ToggleButton, DaySet)>>) {
    let toggle = Rc::new(toggle);
    let row = adw::PreferencesRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inner.set_margin_top(12);
    inner.set_margin_bottom(12);
    inner.set_margin_start(12);
    inner.set_margin_end(12);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    label.set_wrap(true);
    inner.append(&label);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_column_spacing(6);
    flow.set_row_spacing(6);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(7);
    flow.set_hexpand(true);
    let mut buttons: Vec<(gtk::ToggleButton, DaySet)> = Vec::new();
    for (name, bit) in days {
        let button = gtk::ToggleButton::with_label(name);
        button.add_css_class("day-toggle");
        button.set_active(is_on(*bit));
        button.set_hexpand(true);
        let bit = *bit;
        let toggle = toggle.clone();
        button.connect_toggled(move |button| toggle(bit, button.is_active()));
        buttons.push((button.clone(), bit));
        flow.append(&button);
    }
    inner.append(&flow);
    row.set_child(Some(&inner));
    (row, Rc::new(buttons))
}

fn blocked_row(shell: &Rc<RefCell<Shell>>, block: &BlockedTime) -> adw::SwitchRow {
    let id = block.id;
    let row = adw::SwitchRow::new();
    row.set_title(&block.label);
    row.set_subtitle(&block.list_detail());
    row.set_active(block.is_enabled);
    row.connect_active_notify({
        let shell = shell.clone();
        move |row| {
            let on = row.is_active();
            update_prefs(&shell, |prefs| {
                if let Some(block) = prefs.blocked_times.iter_mut().find(|block| block.id == id) {
                    block.is_enabled = on;
                }
            });
        }
    });
    let strength = gtk::DropDown::from_strings(&["If possible", "Always"]);
    strength.set_selected(if block.strength == PreferenceStrength::Required {
        1
    } else {
        0
    });
    strength.set_valign(gtk::Align::Center);
    strength.connect_selected_notify({
        let shell = shell.clone();
        move |menu| {
            let value = if menu.selected() == 1 {
                PreferenceStrength::Required
            } else {
                PreferenceStrength::Preferred
            };
            update_prefs(&shell, |prefs| {
                if let Some(block) = prefs.blocked_times.iter_mut().find(|block| block.id == id) {
                    block.strength = value;
                }
            });
        }
    });
    let remove = gtk::Button::with_label("Remove");
    remove.add_css_class("flat");
    remove.set_valign(gtk::Align::Center);
    remove.connect_clicked({
        let shell = shell.clone();
        move |_| {
            update_prefs(&shell, |prefs| {
                prefs.blocked_times.retain(|block| block.id != id)
            })
        }
    });
    row.add_suffix(&strength);
    row.add_suffix(&remove);
    row
}

fn split_names(text: &str) -> Vec<String> {
    text.split(',')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

pub fn load_dismissed() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dismissed_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn dismissed_path() -> std::path::PathBuf {
    store::data_dir().join("dismissed-equivalences.txt")
}

fn save_dismissed(pairs: &[String]) {
    let path = dismissed_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, pairs.join("\n"));
}

fn pair_id(a: &CourseKey, b: &CourseKey) -> String {
    if a < b {
        format!("{a}|{b}")
    } else {
        format!("{b}|{a}")
    }
}

fn unbroken(key: &CourseKey) -> String {
    format!("{}\u{00A0}{}", key.subject, key.number)
}

pub fn equivalence_section(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let prompts = equivalence_prompts(shell);
    if prompts.is_empty() {
        return root.upcast();
    }
    root.append(&heading_label("Did this count?"));
    root.append(&dim_label("A retired course may be the same class under a new number. Saying yes counts it everywhere, including prerequisites."));
    for (taken, counts_as, evidence) in prompts {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("card");
        card.add_css_class("semester-card");
        card.append(&heading_label(&format!(
            "Does {} count as {}?",
            unbroken(&taken),
            unbroken(&counts_as)
        )));
        if !evidence.is_empty() {
            card.append(&selectable(&evidence));
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let yes = gtk::Button::with_label("Yes, it counts");
        let no = gtk::Button::with_label("No");
        yes.connect_clicked({
            let shell = shell.clone();
            let taken = taken.clone();
            let counts_as = counts_as.clone();
            move |_| declare_equivalence(&shell, &taken, &counts_as)
        });
        no.connect_clicked({
            let shell = shell.clone();
            let taken = taken.clone();
            let counts_as = counts_as.clone();
            move |_| dismiss_equivalence(&shell, &taken, &counts_as)
        });
        actions.append(&yes);
        actions.append(&no);
        card.append(&actions);
        root.append(&card);
    }
    root.upcast()
}

fn equivalence_prompts(shell: &Rc<RefCell<Shell>>) -> Vec<(CourseKey, CourseKey, String)> {
    let shell_ref = shell.borrow();
    let state = shell_ref.state.borrow();
    let Some(plan_id) = state
        .degree
        .or_else(|| state.data.degree_plans.first().map(|plan| plan.id))
    else {
        return vec![];
    };
    let Some(plan) = state
        .data
        .degree_plans
        .iter()
        .find(|plan| plan.id == plan_id)
    else {
        return vec![];
    };
    let known = CourseEquivalences::from_cross_listed(&state.index).merging(
        &CourseEquivalences::from_declarations(&plan.course_equivalences),
    );
    let dismissed: HashSet<String> = state.dismissed_pairs.iter().cloned().collect();
    let mut plan_titles = HashMap::new();
    let mut retired = Vec::new();
    for course in plan.terms.iter().flat_map(|term| term.courses.iter()) {
        let Some(key) = course.key.clone() else {
            continue;
        };
        plan_titles.insert(key.clone(), course.title.clone());
        if state.index.course(&key).is_none() && !retired.contains(&key) {
            retired.push(key);
        }
    }
    let title_candidates: Vec<CourseKey> = state
        .index
        .courses
        .iter()
        .map(|course| course.key())
        .collect();
    let mut prompts = Vec::new();
    for key in retired {
        let pairs: Vec<(CourseKey, Requisite)> = state
            .index
            .courses
            .iter()
            .filter_map(|course| {
                course
                    .prereqs
                    .clone()
                    .map(|requisite| (course.key(), requisite))
            })
            .filter(|(_, requisite)| {
                requisite
                    .referenced_courses()
                    .iter()
                    .any(|named| named == &key)
            })
            .collect();
        let found = suggestions::suggestions(
            &key,
            &pairs,
            |candidate| state.index.course(candidate).is_some(),
            &known,
            2,
            0.5,
            1,
            |candidate| {
                state
                    .index
                    .course(candidate)
                    .map(|course| course.title.clone())
                    .or_else(|| plan_titles.get(candidate).cloned())
                    .or_else(|| {
                        state
                            .course_notes
                            .get(candidate)
                            .map(|info| info.title.clone())
                    })
            },
            &title_candidates,
            titles_minimum(),
        );
        if let Some(best) = found.into_iter().find(|item| {
            !dismissed.contains(&pair_id(&key, &item.candidate))
                && !known.are_equivalent(&key, &item.candidate)
        }) {
            let evidence = best.evidence();
            prompts.push((key, best.candidate, evidence));
        }
        if prompts.len() == 3 {
            break;
        }
    }
    prompts
}

fn titles_minimum() -> f64 {
    termorganize_core::titles::MINIMUM_SIMILARITY
}

fn declare_equivalence(shell: &Rc<RefCell<Shell>>, taken: &CourseKey, counts_as: &CourseKey) {
    {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
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
        let known = CourseEquivalences::from_declarations(&plan.course_equivalences);
        if taken != counts_as && !known.are_equivalent(taken, counts_as) {
            plan.course_equivalences.push(CourseEquivalence {
                id: Uuid::new_v4(),
                course: taken.clone(),
                counts_as: counts_as.clone(),
                note: String::new(),
            });
            plan.updated_at = Some(now_secs().to_string());
            persist(&state.data);
        }
    }
    refresh(shell);
}

fn dismiss_equivalence(shell: &Rc<RefCell<Shell>>, taken: &CourseKey, counts_as: &CourseKey) {
    let id = pair_id(taken, counts_as);
    let shell_ref = shell.borrow();
    let mut state = shell_ref.state.borrow_mut();
    state.dismissed_pairs.retain(|pair| pair != &id);
    state.dismissed_pairs.push(id);
    if state.dismissed_pairs.len() > 200 {
        let extra = state.dismissed_pairs.len() - 200;
        state.dismissed_pairs.drain(0..extra);
    }
    save_dismissed(&state.dismissed_pairs);
    drop(state);
    drop(shell_ref);
    refresh(shell);
}

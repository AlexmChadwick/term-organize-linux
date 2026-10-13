//! Term menu, course windows, prerequisite trees, semester drag-and-drop,
//! petition questions, and the rest of the preferences survey.

use super::*;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::catalog;
use termorganize_core::course_info::{self, CourseInfoRepository};
use termorganize_core::editing;
use termorganize_core::offering::{OfferingHistory, OfferingRepository, OfferingSource};
use termorganize_core::outline::{GroupKind, ItemKind, PathContent, RequisiteItem, RequisiteLayout, RequisitePart};
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
    fn offered_courses(&self, _term: &str, _courses: &[CourseKey]) -> Result<HashSet<CourseKey>, String> {
        Err("offline".into())
    }
}

pub fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".req-chip { padding: 2px 4px; }
         .req-grey { opacity: 0.5; }
         .req-branch { margin-left: 12px; padding: 4px; }
         .semester-card { padding: 8px; }",
    );
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
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
        map.entry(term.code.clone()).or_insert_with(|| term_label(term));
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
    let current = shell.borrow().state.borrow().index.catalog.term.code.clone();
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
    shell.borrow().state.borrow_mut().term_codes = terms.iter().map(|(code, _)| code.clone()).collect();
    shell.borrow().state.borrow_mut().term_labels = terms.iter().map(|(_, label)| label.clone()).collect();
    let labels: Vec<&str> = terms.iter().map(|(_, label)| label.as_str()).collect();
    let model = gtk::StringList::new(&labels);
    let menu = shell.borrow().term_menu.clone();
    menu.set_model(Some(&model));
    let index = terms.iter().position(|(code, _)| code == selected).unwrap_or(0) as u32;
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
            let current = shell.borrow().state.borrow().index.catalog.term.code.clone();
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
    toast(shell, &format!("Downloading term {code} from Banner. This can take a minute."));
    let code = code.to_string();
    let (knowledge, subjects, campuses, attributes, description) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let knowledge = state.index.catalog.courses.iter().map(|course| (course.key(), course.clone())).collect::<HashMap<_, _>>();
        let description = state
            .term_codes
            .iter()
            .position(|item| item == &code)
            .and_then(|index| state.term_labels.get(index))
            .map(|label| label.trim_end_matches(&format!(" · {code}")).trim().to_string())
            .unwrap_or_default();
        (knowledge, state.index.catalog.subjects.clone(), state.index.catalog.campuses.clone(), state.index.catalog.attributes.clone(), description)
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
            let term = Term { code: code.clone(), banner_description: description };
            let now = now_secs();
            let merged = catalog::merge(existing.as_ref(), &sections, term, subjects, campuses, attributes, &knowledge, now);
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
            let current = shell.borrow().state.borrow().index.catalog.term.code.clone();
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
    let scroll = gtk::ScrolledWindow::builder().child(&page).build();
    let title = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        state.index.course(key).map(|course| format!("{} · {}", course.code(), course.title)).unwrap_or_else(|| key.to_string())
    };
    let window = adw::Window::builder().title(&title).default_width(760).default_height(680).content(&scroll).build();
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
    let window = adw::Window::builder().title(name).default_width(720).default_height(640).content(&page).build();
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
    source.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&payload.to_value())));
    widget.add_controller(source);
}

pub fn requisite_block(shell: &Rc<RefCell<Shell>>, requisite: &Requisite) -> gtk::Widget {
    let (offered, completed, equivalences) = {
        let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
        let offered: HashSet<CourseKey> = state.index.courses.iter().map(|course| course.key()).collect();
        let plan = state.data.primary_degree_plan();
        let completed: HashSet<CourseKey> = plan.map(|plan| plan.completed_keys().into_iter().collect()).unwrap_or_default();
        let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(&CourseEquivalences::from_declarations(
            plan.map(|plan| plan.course_equivalences.as_slice()).unwrap_or(&[]),
        ));
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
    box_.append(&dim_label(&layout.spoken_description("Prerequisites")));
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
        PathContent::Checklist(list) => draw_layout(shell, &RequisiteLayout::Checklist(list.clone())),
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
    let tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
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
    let Some(plan_id) = state.degree.filter(|id| state.data.degree_plans.iter().any(|plan| plan.id == *id)).or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else {
        return vec![];
    };
    let Some(plan) = state.data.degree_plans.iter().find(|plan| plan.id == plan_id) else { return vec![] };
    let history = &state.offering;
    let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(&CourseEquivalences::from_declarations(&plan.course_equivalences));
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
            courses.push(CourseRow { id: course.id, key: course.key.clone(), label: format!("{} {}", course.label, course.title), detail });
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
        views.push(SemesterSnap { id: term.id, title: format!("{} · {:.1} credits", term.name(), term.credits()), courses, suggestions });
    }
    views
}

fn ensure_local_offering(shell: &Rc<RefCell<Shell>>) {
    let ready = !shell.borrow().state.borrow().offering.checked_terms.is_empty();
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
    for course in plan.completed.iter().chain(plan.terms.iter().flat_map(|term| term.courses.iter())) {
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
    let terms = indexes.iter().map(|index| index.catalog.term.clone()).collect();
    let mut repo = OfferingRepository::new(LocalTerms { terms }, store::data_dir().join("offering-history.json"), now_secs());
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
        let mut offering_repo = OfferingRepository::new(BannerClient::default(), store::data_dir().join("offering-history.json"), now);
        let history = offering_repo.history(&keys, &indexes);
        let missing: Vec<CourseKey> = keys
            .iter()
            .filter(|key| indexes.iter().all(|index| index.course(key).is_none_or(|course| !course.details_loaded)))
            .cloned()
            .collect();
        let mut info_repo = CourseInfoRepository::new(BannerClient::default(), store::data_dir().join("course-info.json"), now);
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
            toast(&shell, &format!("Checked {checked} terms. {found} older catalog entries."));
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
    let keys: Vec<Option<CourseKey>> = semester.courses.iter().map(|course| course.key.clone()).collect();
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

fn course_drag_row(shell: &Rc<RefCell<Shell>>, term_id: Uuid, course: &CourseRow, position: i32) -> gtk::ListBoxRow {
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
        entries.push(super::menu_action("Open", move || open_course_window(&shell_open, &key_open)));
        let shell_copy = shell.clone();
        let code = key.to_string();
        entries.push(super::menu_action("Copy code", move || super::copy_text(&shell_copy, &code, "Course code copied")));
    }
    let shell_remove = shell.clone();
    let course_id = course.id;
    entries.push(super::menu_action("Remove", move || super::remove_degree_course(&shell_remove, course_id)));
    super::attach_context_menu(&row, entries);
    let source = gtk::DragSource::new();
    source.set_actions(gtk::gdk::DragAction::MOVE);
    let payload = format!("id:{}", course.id);
    source.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&payload.to_value())));
    row.add_controller(source);
    row.set_child(Some(&box_));
    attach_semester_drop(&row, shell, term_id, position);
    row
}

fn attach_semester_drop(widget: &impl IsA<gtk::Widget>, shell: &Rc<RefCell<Shell>>, term_id: Uuid, position: i32) {
    let target = gtk::DropTarget::new(glib::Type::STRING, gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE);
    target.connect_drop({
        let shell = shell.clone();
        move |_, value, _, _| {
            let Ok(text) = value.get::<String>() else { return false };
            apply_drop(&shell, &text, term_id, position)
        }
    });
    widget.add_controller(target);
}

fn apply_drop(shell: &Rc<RefCell<Shell>>, payload: &str, term_id: Uuid, position: i32) -> bool {
    let added = if let Some(raw) = payload.strip_prefix("key:") {
        let Some(key) = CourseKey::parse(raw) else { return false };
        let (title, credits) = {
            let shell_ref = shell.borrow();
        let state = shell_ref.state.borrow();
            state.index.course(&key).map(|course| (course.title.clone(), course.max_credits)).unwrap_or_else(|| (key.to_string(), 4.0))
        };
        Some(DegreePlanCourse::catalog(key, title, credits))
    } else {
        None
    };
    let moved = {
        let shell_ref = shell.borrow();
        let mut state = shell_ref.state.borrow_mut();
        let Some(plan_id) = state.degree.or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else { return false };
        let Some(plan) = state.data.degree_plans.iter_mut().find(|plan| plan.id == plan_id) else { return false };
        if let Some(id) = payload.strip_prefix("id:").and_then(|raw| Uuid::parse_str(raw).ok()) {
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
        let Some(plan_id) = state.degree.or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else { return };
        let Some(plan) = state.data.degree_plans.iter_mut().find(|plan| plan.id == plan_id) else { return };
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
        let Some(plan) = state.degree.and_then(|id| state.data.degree_plans.iter().find(|plan| plan.id == id)).or_else(|| state.data.degree_plans.first()) else {
            return root.upcast();
        };
        let Some(requirements) = state.requirements.get(&plan.id) else { return root.upcast() };
        let equivalences = CourseEquivalences::from_cross_listed(&state.index).merging(&CourseEquivalences::from_declarations(&plan.course_equivalences));
        let questions = petition::questions(plan, requirements, &equivalences, None);
        let audit = DegreeAudit::run(plan, requirements, &equivalences, &DegreeCourseAttributes::none());
        let mut misses = Vec::new();
        for result in audit.rule_results() {
            for miss in audit.near_misses(&result.rule.id) {
                misses.push(format!("{} · {} · {}", result.rule.display_label(), miss.course.key, miss.reason));
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

fn petition_card(shell: &Rc<RefCell<Shell>>, question: &petition::DegreePetitionQuestion) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.add_css_class("semester-card");
    card.append(&heading_label(&question.question()));
    card.append(&selectable(&question.explanation()));
    let minimum = CourseNumberChoice::suggested_minimum(&question.ranges, None);
    let entry = gtk::Entry::builder().placeholder_text("Minimum, like 2000").text(minimum.to_string()).build();
    let group = gtk::CheckButton::with_label(&CourseNumberChoice::SameAsRequirement.title(&question.like_title));
    group.set_tooltip_text(Some(&CourseNumberChoice::SameAsRequirement.detail(&question.ranges, minimum)));
    let any = gtk::CheckButton::with_label(&CourseNumberChoice::AnyNumber.title(&question.like_title));
    any.set_group(Some(&group));
    any.set_tooltip_text(Some(&CourseNumberChoice::AnyNumber.detail(&question.ranges, minimum)));
    let least = gtk::CheckButton::with_label(&CourseNumberChoice::AtLeast.title(&question.like_title));
    least.set_group(Some(&group));
    least.set_tooltip_text(Some(&CourseNumberChoice::AtLeast.detail(&question.ranges, minimum)));
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
    card.append(&dim_label(&CourseNumberChoice::SameAsRequirement.detail(&question.ranges, minimum)));
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
        let Some(plan_id) = state.degree.or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else { return };
        let Some(plan) = state.data.degree_plans.iter_mut().find(|plan| plan.id == plan_id) else { return };
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
        current_plan_id(&state).and_then(|id| state.data.plans.iter().find(|plan| plan.id == id)).and_then(|plan| plan.preferences.clone()).unwrap_or_default()
    };
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&dim_label("Checked days are days off. Required drops sections. Preferred only changes the ranking. Press Enter in a text field to apply it."));

    box_.append(&heading_label("Time of day"));
    for band in TimeBand::ALL {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&format!("{} · {}", band.title(), band.detail())));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let menu = gtk::DropDown::from_strings(&["Avoid", "Okay", "Prefer"]);
        menu.set_selected(match prefs.band(band) {
            TimePreference::Avoid => 0,
            TimePreference::Okay => 1,
            TimePreference::Prefer => 2,
        });
        menu.connect_selected_notify({
            let shell = shell.clone();
            move |menu| {
                let pref = match menu.selected() {
                    0 => TimePreference::Avoid,
                    2 => TimePreference::Prefer,
                    _ => TimePreference::Okay,
                };
                update_prefs(&shell, |prefs| prefs.set_band(band, pref));
            }
        });
        row.append(&label);
        row.append(&menu);
        box_.append(&row);
    }

    let days = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    days.append(&dim_label("Days off"));
    for (label, bit) in [("Mon", DaySet::MONDAY), ("Tue", DaySet::TUESDAY), ("Wed", DaySet::WEDNESDAY), ("Thu", DaySet::THURSDAY), ("Fri", DaySet::FRIDAY), ("Sat", DaySet::SATURDAY), ("Sun", DaySet::SUNDAY)] {
        let button = gtk::CheckButton::with_label(label);
        button.set_active(prefs.days_off.contains(bit));
        button.connect_toggled({
            let shell = shell.clone();
            move |button| {
                update_prefs(&shell, |prefs| {
                    if button.is_active() {
                        prefs.days_off.insert(bit);
                    } else {
                        prefs.days_off.remove(bit);
                    }
                });
            }
        });
        days.append(&button);
    }
    box_.append(&days);
    box_.append(&strength_check(shell, "Days off are required", prefs.days_off_strength, |prefs, strength| prefs.days_off_strength = strength));

    let limits = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    limits.append(&clock_entry(shell, "Nothing before", prefs.earliest_start.as_ref().map(|limit| clock_label(limit.minutes)).unwrap_or_default(), true));
    limits.append(&clock_entry(shell, "Nothing after", prefs.latest_end.as_ref().map(|limit| clock_label(limit.minutes)).unwrap_or_default(), false));
    box_.append(&limits);
    box_.append(&strength_check(shell, "Earliest start is required", prefs.earliest_start.as_ref().map(|limit| limit.strength).unwrap_or(PreferenceStrength::Required), |prefs, strength| {
        if let Some(limit) = &mut prefs.earliest_start {
            limit.strength = strength;
        }
    }));
    box_.append(&strength_check(shell, "Latest end is required", prefs.latest_end.as_ref().map(|limit| limit.strength).unwrap_or(PreferenceStrength::Required), |prefs, strength| {
        if let Some(limit) = &mut prefs.latest_end {
            limit.strength = strength;
        }
    }));

    box_.append(&enum_menu(shell, "Day layout", &["No preference", "Fewer days", "Spread out"], match prefs.day_layout {
        DayLayout::NoPreference => 0,
        DayLayout::FewerDays => 1,
        DayLayout::SpreadOut => 2,
    }, |prefs, index| {
        prefs.day_layout = match index {
            1 => DayLayout::FewerDays,
            2 => DayLayout::SpreadOut,
            _ => DayLayout::NoPreference,
        };
    }));
    box_.append(&enum_menu(shell, "Gaps", &["Compact", "No preference", "Gaps are fine"], match prefs.gaps {
        GapPreference::Compact => 0,
        GapPreference::NoPreference => 1,
        GapPreference::Fine => 2,
    }, |prefs, index| {
        prefs.gaps = match index {
            0 => GapPreference::Compact,
            2 => GapPreference::Fine,
            _ => GapPreference::NoPreference,
        };
    }));
    box_.append(&enum_menu(shell, "Breaks", &["Back-to-back is fine", "Avoid back-to-back", "At least a break"], match prefs.breaks {
        BreakPreference::Fine => 0,
        BreakPreference::AvoidBackToBack => 1,
        BreakPreference::AtLeast => 2,
    }, |prefs, index| {
        prefs.breaks = match index {
            1 => BreakPreference::AvoidBackToBack,
            2 => BreakPreference::AtLeast,
            _ => BreakPreference::Fine,
        };
    }));

    let numbers = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    numbers.append(&number_entry(shell, "Max classes / day", prefs.max_classes_per_day.map(|n| n.to_string()).unwrap_or_default(), |prefs, value| {
        prefs.max_classes_per_day = value;
    }));
    numbers.append(&number_entry(shell, "Max minutes / day", prefs.max_minutes_per_day.map(|n| n.to_string()).unwrap_or_default(), |prefs, value| {
        prefs.max_minutes_per_day = value;
    }));
    numbers.append(&number_entry(shell, "Minimum break minutes", prefs.minimum_break_minutes.to_string(), |prefs, value| {
        if let Some(value) = value {
            prefs.minimum_break_minutes = value;
        }
    }));
    box_.append(&numbers);
    box_.append(&strength_check(shell, "Minimum break is required", prefs.minimum_break_strength, |prefs, strength| prefs.minimum_break_strength = strength));

    let lunch = gtk::CheckButton::with_label("Keep a lunch break");
    lunch.set_active(prefs.lunch.is_some());
    lunch.connect_toggled({
        let shell = shell.clone();
        move |button| {
            update_prefs(&shell, |prefs| prefs.lunch = if button.is_active() { Some(LunchBreak::default()) } else { None });
        }
    });
    box_.append(&lunch);

    box_.append(&heading_label("Blocked times"));
    for block in &prefs.blocked_times {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let enabled = gtk::CheckButton::with_label(&format!("{} · {}", block.label, block.list_detail()));
        enabled.set_active(block.is_enabled);
        let id = block.id;
        enabled.connect_toggled({
            let shell = shell.clone();
            move |button| {
                update_prefs(&shell, |prefs| {
                    if let Some(block) = prefs.blocked_times.iter_mut().find(|block| block.id == id) {
                        block.is_enabled = button.is_active();
                    }
                });
            }
        });
        let strength = gtk::DropDown::from_strings(&["If possible", "Always"]);
        strength.set_selected(if block.strength == PreferenceStrength::Required { 1 } else { 0 });
        strength.connect_selected_notify({
            let shell = shell.clone();
            move |menu| {
                let value = if menu.selected() == 1 { PreferenceStrength::Required } else { PreferenceStrength::Preferred };
                update_prefs(&shell, |prefs| {
                    if let Some(block) = prefs.blocked_times.iter_mut().find(|block| block.id == id) {
                        block.strength = value;
                    }
                });
            }
        });
        let remove = gtk::Button::with_label("Remove");
        remove.connect_clicked({
            let shell = shell.clone();
            move |_| {
                update_prefs(&shell, |prefs| prefs.blocked_times.retain(|block| block.id != id));
            }
        });
        row.append(&enabled);
        row.append(&strength);
        row.append(&remove);
        box_.append(&row);
    }
    box_.append(&blocked_form(shell));

    box_.append(&heading_label("Format, campus, seats"));
    box_.append(&enum_menu(shell, "Format", &["No format preference", "In person", "Hybrid", "Online"], match prefs.format {
        FormatPreference::NoPreference => 0,
        FormatPreference::InPerson => 1,
        FormatPreference::Hybrid => 2,
        FormatPreference::Online => 3,
    }, |prefs, index| {
        prefs.format = match index {
            1 => FormatPreference::InPerson,
            2 => FormatPreference::Hybrid,
            3 => FormatPreference::Online,
            _ => FormatPreference::NoPreference,
        };
    }));
    box_.append(&strength_check(shell, "Format is required", prefs.format_strength, |prefs, strength| prefs.format_strength = strength));
    let campus = gtk::Entry::builder().placeholder_text("Campus, like Boston").text(prefs.campus.clone().unwrap_or_default()).hexpand(true).build();
    campus.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let text = entry.text().trim().to_string();
            update_prefs(&shell, |prefs| prefs.campus = if text.is_empty() { None } else { Some(text) });
        }
    });
    box_.append(&campus);
    box_.append(&strength_check(shell, "Campus is required", prefs.campus_strength, |prefs, strength| prefs.campus_strength = strength));
    box_.append(&enum_menu(shell, "Seats", &["Any seats", "Waitlist is fine", "Open seats only"], match prefs.seats {
        SeatPolicy::Any => 0,
        SeatPolicy::WaitlistOk => 1,
        SeatPolicy::OpenOnly => 2,
    }, |prefs, index| {
        prefs.seats = match index {
            1 => SeatPolicy::WaitlistOk,
            2 => SeatPolicy::OpenOnly,
            _ => SeatPolicy::Any,
        };
    }));

    box_.append(&heading_label("Instructors and credits"));
    let preferred = gtk::Entry::builder().placeholder_text("Preferred instructors, comma separated").text(prefs.preferred_instructors.join(", ")).build();
    preferred.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let names = split_names(&entry.text());
            update_prefs(&shell, |prefs| prefs.preferred_instructors = names);
        }
    });
    let avoided = gtk::Entry::builder().placeholder_text("Avoided instructors, comma separated").text(prefs.avoided_instructors.join(", ")).build();
    avoided.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let names = split_names(&entry.text());
            update_prefs(&shell, |prefs| prefs.avoided_instructors = names);
        }
    });
    box_.append(&preferred);
    box_.append(&avoided);
    let credits = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let lower = gtk::Entry::builder().placeholder_text("Min credits").text(prefs.credit_target.as_ref().map(|t| t.lower.to_string()).unwrap_or_default()).build();
    let upper = gtk::Entry::builder().placeholder_text("Max credits").text(prefs.credit_target.as_ref().map(|t| t.upper.to_string()).unwrap_or_default()).build();
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
    lower.connect_activate({
        let apply_credits = apply_credits.clone();
        move |_| apply_credits()
    });
    upper.connect_activate({
        let apply_credits = apply_credits.clone();
        move |_| apply_credits()
    });
    credits.append(&lower);
    credits.append(&upper);
    box_.append(&credits);
    box_.upcast()
}

fn clock_entry(shell: &Rc<RefCell<Shell>>, placeholder: &str, text: String, earliest: bool) -> gtk::Widget {
    let entry = gtk::Entry::builder().placeholder_text(placeholder).text(&text).build();
    entry.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let minutes = parse_clock(&entry.text());
            update_prefs(&shell, |prefs| {
                let limit = minutes.map(|minutes| TimeLimit { minutes, strength: PreferenceStrength::Required });
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
    });
    entry.upcast()
}

fn number_entry(shell: &Rc<RefCell<Shell>>, placeholder: &str, text: String, apply: impl Fn(&mut SchedulePreferences, Option<i32>) + 'static) -> gtk::Widget {
    let entry = gtk::Entry::builder().placeholder_text(placeholder).text(&text).build();
    entry.connect_activate({
        let shell = shell.clone();
        move |entry| {
            let value = entry.text().trim().parse::<i32>().ok();
            update_prefs(&shell, |prefs| apply(prefs, value));
        }
    });
    entry.upcast()
}

fn strength_check(shell: &Rc<RefCell<Shell>>, label: &str, strength: PreferenceStrength, apply: impl Fn(&mut SchedulePreferences, PreferenceStrength) + 'static) -> gtk::Widget {
    let button = gtk::CheckButton::with_label(label);
    button.set_active(strength == PreferenceStrength::Required);
    button.connect_toggled({
        let shell = shell.clone();
        move |button| {
            let strength = if button.is_active() { PreferenceStrength::Required } else { PreferenceStrength::Preferred };
            update_prefs(&shell, |prefs| apply(prefs, strength));
        }
    });
    button.upcast()
}

fn enum_menu(shell: &Rc<RefCell<Shell>>, _title: &str, labels: &[&str], selected: u32, apply: impl Fn(&mut SchedulePreferences, u32) + 'static) -> gtk::Widget {
    let menu = gtk::DropDown::from_strings(labels);
    menu.set_selected(selected);
    menu.connect_selected_notify({
        let shell = shell.clone();
        move |menu| {
            let index = menu.selected();
            update_prefs(&shell, |prefs| apply(prefs, index));
        }
    });
    menu.upcast()
}

fn blocked_form(shell: &Rc<RefCell<Shell>>) -> gtk::Widget {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let label = gtk::Entry::builder().placeholder_text("Blocked time, like Work").build();
    let days = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let mut checks = Vec::new();
    for (name, bit) in [("Mon", DaySet::MONDAY), ("Tue", DaySet::TUESDAY), ("Wed", DaySet::WEDNESDAY), ("Thu", DaySet::THURSDAY), ("Fri", DaySet::FRIDAY)] {
        let button = gtk::CheckButton::with_label(name);
        days.append(&button);
        checks.push((button, bit));
    }
    let start = gtk::Entry::builder().placeholder_text("Start 9:00").text("9:00").build();
    let end = gtk::Entry::builder().placeholder_text("End 12:00").text("12:00").build();
    let required = gtk::CheckButton::with_label("Required");
    required.set_active(true);
    let add = gtk::Button::with_label("Add blocked time");
    let times = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    times.append(&start);
    times.append(&end);
    times.append(&required);
    times.append(&add);
    box_.append(&label);
    box_.append(&days);
    box_.append(&times);
    add.connect_clicked({
        let shell = shell.clone();
        let checks = checks.clone();
        move |_| {
            let mut dayset = DaySet(0);
            for (button, bit) in &checks {
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
                label: if name.is_empty() { "Blocked time".into() } else { name },
                days: dayset,
                start: start_m,
                end: end_m,
                strength: if required.is_active() { PreferenceStrength::Required } else { PreferenceStrength::Preferred },
                is_enabled: true,
            };
            if !block.is_valid() {
                toast(&shell, "Pick at least one day, and make the end later than the start.");
                return;
            }
            update_prefs(&shell, |prefs| prefs.blocked_times.push(block));
        }
    });
    box_.upcast()
}

fn split_names(text: &str) -> Vec<String> {
    text.split(',').map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect()
}

pub fn load_dismissed() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dismissed_path()) else { return Vec::new() };
    text.lines().filter(|line| !line.is_empty()).map(|line| line.to_string()).collect()
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
    if a < b { format!("{a}|{b}") } else { format!("{b}|{a}") }
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
        card.append(&heading_label(&format!("Does {} count as {}?", unbroken(&taken), unbroken(&counts_as))));
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
    let Some(plan_id) = state.degree.or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else { return vec![] };
    let Some(plan) = state.data.degree_plans.iter().find(|plan| plan.id == plan_id) else { return vec![] };
    let known = CourseEquivalences::from_cross_listed(&state.index).merging(&CourseEquivalences::from_declarations(&plan.course_equivalences));
    let dismissed: HashSet<String> = state.dismissed_pairs.iter().cloned().collect();
    let mut plan_titles = HashMap::new();
    let mut retired = Vec::new();
    for course in plan.terms.iter().flat_map(|term| term.courses.iter()) {
        let Some(key) = course.key.clone() else { continue };
        plan_titles.insert(key.clone(), course.title.clone());
        if state.index.course(&key).is_none() && !retired.contains(&key) {
            retired.push(key);
        }
    }
    let title_candidates: Vec<CourseKey> = state.index.courses.iter().map(|course| course.key()).collect();
    let mut prompts = Vec::new();
    for key in retired {
        let pairs: Vec<(CourseKey, Requisite)> = state
            .index
            .courses
            .iter()
            .filter_map(|course| course.prereqs.clone().map(|requisite| (course.key(), requisite)))
            .filter(|(_, requisite)| requisite.referenced_courses().iter().any(|named| named == &key))
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
                state.index.course(candidate).map(|course| course.title.clone()).or_else(|| plan_titles.get(candidate).cloned()).or_else(|| state.course_notes.get(candidate).map(|info| info.title.clone()))
            },
            &title_candidates,
            titles_minimum(),
        );
        if let Some(best) = found.into_iter().find(|item| !dismissed.contains(&pair_id(&key, &item.candidate)) && !known.are_equivalent(&key, &item.candidate)) {
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
        let Some(plan_id) = state.degree.or_else(|| state.data.degree_plans.first().map(|plan| plan.id)) else { return };
        let Some(plan) = state.data.degree_plans.iter_mut().find(|plan| plan.id == plan_id) else { return };
        let known = CourseEquivalences::from_declarations(&plan.course_equivalences);
        if taken != counts_as && !known.are_equivalent(taken, counts_as) {
            plan.course_equivalences.push(CourseEquivalence { id: Uuid::new_v4(), course: taken.clone(), counts_as: counts_as.clone(), note: String::new() });
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

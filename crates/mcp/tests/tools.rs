use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use term_organize_mcp::server::TermOrganize;
use term_organize_mcp::tools;
use term_organize_mcp::{resolve_mcp_data_dir, INSTRUCTIONS};
use termorganize_core::email::is_northeastern_address;
use termorganize_core::model::{CourseKey, DegreePlanCourse, PlanSeason};
use termorganize_core::planfile;
use termorganize_core::store;
use termorganize_core::UserData;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "termorganize-mcp-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fall_course() -> tools::CourseArgs {
    tools::CourseArgs {
        term: "Fall 2026".into(),
        subject: "CS".into(),
        number: "2100".into(),
    }
}

#[test]
fn every_catalog_tool_reads_the_bundled_terms() {
    let terms = tools::list_terms();
    let codes: Vec<&str> = terms["terms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|term| term["term"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["202730", "202710"]);

    let found = tools::search_courses(tools::SearchArgs {
        term: "202710".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("2100".into()),
        attribute: None,
        limit: Some(5),
    })
    .unwrap();
    assert_eq!(found["courses"][0]["subject"], "CS");
    assert_eq!(found["courses"][0]["number"], "2100");
    assert!(found["courses"][0]["title"].as_str().unwrap().len() > 3);
    assert!(found["courses"][0].get("credits").is_some());

    let course = tools::get_course(fall_course()).unwrap();
    let sections = course["sections"].as_array().unwrap();
    assert!(!sections.is_empty());
    let crn = sections[0]["crn"].as_str().unwrap().to_string();
    for section in sections {
        for instructor in section["instructors"].as_array().unwrap() {
            if let Some(email) = instructor["email"].as_str() {
                assert!(is_northeastern_address(email), "{email}");
            }
        }
    }

    let check = tools::check_schedule(tools::CheckArgs {
        term: "Fall 2026".into(),
        crns: vec![crn, "00000".into()],
    })
    .unwrap();
    assert_eq!(check["unknown_crns"][0], "00000");

    let suggested = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![tools::CourseRef {
            subject: "CS".into(),
            number: "2100".into(),
            crn: None,
        }],
        constraints: Some(tools::Constraints {
            no_classes_before: Some("8:00 AM".into()),
            no_classes_after: Some("9:00 PM".into()),
            days_off: vec!["Friday".into()],
        }),
    })
    .unwrap();
    assert!(!suggested["schedules"].as_array().unwrap().is_empty());

    let reorganized = tools::reorganize_plan(
        std::path::Path::new("."),
        tools::ReorganizeArgs {
            plan_id: None,
            plan: Some(tools::PlanInput {
                terms: vec![
                    tools::TermInput {
                        term: "Fall 2026".into(),
                        courses: vec![tools::CourseRef {
                            subject: "CS".into(),
                            number: "2100".into(),
                            crn: None,
                        }],
                    },
                    tools::TermInput {
                        term: "Spring 2027".into(),
                        courses: vec![],
                    },
                ],
            }),
        },
    )
    .unwrap();
    assert!(reorganized["moves"].as_array().unwrap().is_empty());
    assert!(reorganized["unresolved"].as_array().unwrap().is_empty());
}

#[test]
fn bundled_instructor_emails_stay_on_northeastern_domains() {
    let kept = term_organize_mcp::present::kept_email(
        "person@gmail.com faculty@northeastern.edu other@kent.edu",
    )
    .unwrap();
    assert_eq!(kept, "faculty@northeastern.edu");
    for index in term_organize_mcp::catalogs::catalogs() {
        for section in &index.catalog.sections {
            let value = term_organize_mcp::present::section_json(section);
            for instructor in value["instructors"].as_array().unwrap() {
                if let Some(email) = instructor["email"].as_str() {
                    assert!(
                        is_northeastern_address(email),
                        "{} leaked {email}",
                        section.crn
                    );
                    let domain = email.rsplit_once('@').unwrap().1.to_ascii_lowercase();
                    assert!(
                        domain == "northeastern.edu" || domain == "neu.edu",
                        "{} leaked subdomain {email}",
                        section.crn
                    );
                    assert!(!email.to_ascii_lowercase().contains("husky.neu.edu"));
                    assert!(!email.to_ascii_lowercase().contains("ccs.neu.edu"));
                }
            }
        }
    }
}

#[test]
fn write_tools_round_trip_through_the_app_loader() {
    let dir = temp_dir("writes");
    let created = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "MCP test".into(),
            terms: Some(vec!["Fall 2026".into(), "Spring 2027".into()]),
        },
    )
    .unwrap();
    assert_eq!(created["revision"], 1);
    assert_eq!(created["name"], "MCP test");
    let id = created["id"].as_str().unwrap().to_string();
    let loaded = store::load_user(&dir);
    assert_eq!(loaded.degree_plans[0].name, "MCP test");
    let text = fs::read_to_string(dir.join("user.json")).unwrap();
    let from_app = UserData::from_json_lossy(&text);
    assert_eq!(from_app.degree_plans[0].id.to_string(), id);
    assert!(!created.to_string().contains("studentName"));
    assert!(!created.to_string().contains("studentId"));

    let course = tools::get_course(fall_course()).unwrap();
    let crn = course["sections"][0]["crn"].as_str().unwrap().to_string();
    let added = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "2100".into(),
            crn: Some(crn.clone()),
            expected_revision: 1,
        },
    )
    .unwrap();
    assert_eq!(added["revision"], 2);
    assert_eq!(added["terms"][0]["courses"][0]["subject"], "CS");
    assert_eq!(added["terms"][0]["courses"][0]["crn"], crn);
    assert!(added["terms"][0]["courses"][0]["section"].is_object());
    let round_trip = store::load_user(&dir);
    assert_eq!(
        round_trip.degree_plans[0].terms[0].courses[0]
            .key
            .as_ref()
            .unwrap()
            .number,
        "2100"
    );
    assert!(dir.join("user.json.bak").is_file());

    let renamed = tools::rename_plan(
        &dir,
        tools::RenameArgs {
            plan_id: id.clone(),
            name: "Renamed plan".into(),
            expected_revision: 2,
        },
    )
    .unwrap();
    assert_eq!(renamed["revision"], 3);
    assert_eq!(renamed["name"], "Renamed plan");

    let stale = tools::rename_plan(
        &dir,
        tools::RenameArgs {
            plan_id: id.clone(),
            name: "Stale".into(),
            expected_revision: 2,
        },
    )
    .unwrap_err();
    assert_eq!(stale.error, "revision_conflict");
    assert_eq!(stale.revision, Some(3));
    assert_eq!(store::load_user(&dir).degree_plans[0].name, "Renamed plan");

    let moved = tools::move_course(
        &dir,
        tools::MoveCourseArgs {
            plan_id: id.clone(),
            subject: "CS".into(),
            number: "2100".into(),
            from_term: "Fall 2026".into(),
            to_term: "Spring 2027".into(),
            expected_revision: 3,
        },
    )
    .unwrap();
    assert_eq!(moved["revision"], 4);
    assert!(moved["terms"][0]["courses"].as_array().unwrap().is_empty());
    assert_eq!(moved["terms"][1]["courses"][0]["number"], "2100");

    let spring = tools::get_course(tools::CourseArgs {
        term: "Spring 2027".into(),
        subject: "CS".into(),
        number: "2100".into(),
    })
    .unwrap();
    let spring_crn = spring["sections"][0]["crn"].as_str().unwrap().to_string();
    let sectioned = tools::set_section(
        &dir,
        tools::SetSectionArgs {
            plan_id: id.clone(),
            term: "202730".into(),
            subject: "cs".into(),
            number: "2100".into(),
            crn: spring_crn.clone(),
            expected_revision: 4,
        },
    )
    .unwrap();
    assert_eq!(sectioned["terms"][1]["courses"][0]["crn"], spring_crn);

    let removed = tools::remove_course(
        &dir,
        tools::RemoveCourseArgs {
            plan_id: id.clone(),
            term: "Spring 2027".into(),
            subject: "CS".into(),
            number: "2100".into(),
            expected_revision: sectioned["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert!(removed["terms"][1]["courses"]
        .as_array()
        .unwrap()
        .is_empty());

    let unknown = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "ZZ".into(),
            number: "9999".into(),
            crn: None,
            expected_revision: removed["revision"].as_u64().unwrap(),
        },
    )
    .unwrap_err();
    assert_eq!(unknown.error, "invalid_argument");
    assert!(!unknown.message.is_empty());
    assert!(unknown.to_value()["error"]["code"].as_str().unwrap() == "invalid_argument");
    assert_eq!(
        store::load_user(&dir).degree_plans[0].terms[0]
            .courses
            .len(),
        0
    );

    let refused = tools::delete_plan(
        &dir,
        tools::DeleteArgs {
            plan_id: id.clone(),
            expected_revision: removed["revision"].as_u64().unwrap(),
            confirm: false,
        },
    )
    .unwrap_err();
    assert_eq!(refused.error, "confirm_required");
    assert_eq!(
        tools::list_plans(&dir).unwrap()["plans"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    tools::delete_plan(
        &dir,
        tools::DeleteArgs {
            plan_id: id,
            expected_revision: removed["revision"].as_u64().unwrap(),
            confirm: true,
        },
    )
    .unwrap();
    assert!(tools::list_plans(&dir).unwrap()["plans"]
        .as_array()
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn apply_reorganize_saves_a_move_the_app_can_read() {
    let dir = temp_dir("reorg");
    let created = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "Reorder me".into(),
            terms: Some(vec!["Fall 2026".into(), "Spring 2027".into()]),
        },
    )
    .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let spring_only = spring_only_course();
    let uuid = uuid::Uuid::parse_str(&id).unwrap();
    planfile::update_plan(&dir, uuid, 1, |plan| {
        let term = plan
            .terms
            .iter_mut()
            .find(|term| term.season == PlanSeason::Fall)
            .unwrap();
        term.courses
            .push(DegreePlanCourse::catalog(spring_only, "Spring only", 4.0));
        Ok(())
    })
    .unwrap();
    let applied = tools::apply_reorganize(
        &dir,
        tools::RevisionArgs {
            plan_id: id,
            expected_revision: 2,
        },
    )
    .unwrap();
    assert_eq!(applied["moves"].as_array().unwrap().len(), 1);
    assert_eq!(applied["revision"], 3);
    let loaded = store::load_user(&dir);
    let spring = loaded.degree_plans[0]
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Spring)
        .unwrap();
    assert_eq!(spring.courses.len(), 1);
    assert!(loaded.degree_plans[0].terms[0].courses.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_only_hides_writes_and_annotations_match_the_spec() {
    let dir = temp_dir("ann");
    let server = TermOrganize::new(dir.clone(), false);
    let tools = server.tool_definitions();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name.as_ref() == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    for name in [
        "list_terms",
        "search_courses",
        "get_course",
        "check_schedule",
        "suggest_schedule",
        "reorganize_plan",
        "list_plans",
        "get_plan",
    ] {
        let annotations = tool(name).annotations.clone().unwrap();
        assert_eq!(annotations.read_only_hint, Some(true), "{name}");
        assert_eq!(annotations.open_world_hint, Some(false), "{name}");
    }
    for name in ["delete_plan", "remove_course", "apply_reorganize"] {
        let annotations = tool(name).annotations.clone().unwrap();
        assert_eq!(annotations.destructive_hint, Some(true), "{name}");
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
    }
    for name in [
        "create_plan",
        "rename_plan",
        "add_course",
        "move_course",
        "set_section",
    ] {
        let annotations = tool(name).annotations.clone().unwrap();
        assert_eq!(annotations.destructive_hint, Some(false), "{name}");
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
    }
    let hidden = TermOrganize::new(dir.clone(), true);
    let hidden_tools = hidden.tool_definitions();
    let names: Vec<&str> = hidden_tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(names.contains(&"list_plans"));
    assert!(!names.contains(&"create_plan"));
    assert!(!names.contains(&"delete_plan"));
    assert!(INSTRUCTIONS.contains("Not affiliated"));
    assert_eq!(
        resolve_mcp_data_dir(Some(dir.as_path()), Some("/tmp/ignored")),
        dir
    );
    assert_eq!(
        resolve_mcp_data_dir(None, Some(" /tmp/mcp-data ")),
        PathBuf::from("/tmp/mcp-data")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shared_reorganize_fixture_matches_the_bundled_catalog() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mcp-reorganize-fixture.json");
    let fixture: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("fixture")).expect("fixture json");
    let plan: tools::PlanInput = serde_json::from_value(fixture["plan"].clone()).expect("plan");
    let outcome = tools::reorganize_plan(
        std::path::Path::new("."),
        tools::ReorganizeArgs {
            plan_id: None,
            plan: Some(plan),
        },
    )
    .unwrap();
    assert_eq!(outcome["moves"], fixture["expected"]["moves"]);
    assert_eq!(outcome["unresolved"], fixture["expected"]["unresolved"]);
    let spring = outcome["plan"]["terms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|term| term["term"] == "202730")
        .unwrap();
    let numbers: Vec<&str> = spring["courses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|course| course["number"].as_str().unwrap())
        .collect();
    assert_eq!(numbers, ["2701"]);
}

#[test]
fn bare_hour_is_an_invalid_time_and_eight_am_is_parsed() {
    let rejected = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![tools::CourseRef {
            subject: "CS".into(),
            number: "2100".into(),
            crn: None,
        }],
        constraints: Some(tools::Constraints {
            no_classes_before: Some("8".into()),
            no_classes_after: None,
            days_off: vec![],
        }),
    })
    .unwrap_err();
    assert_eq!(rejected.error, "invalid_argument");
    assert!(!rejected.message.is_empty());

    let parsed = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![tools::CourseRef {
            subject: "CS".into(),
            number: "2100".into(),
            crn: None,
        }],
        constraints: Some(tools::Constraints {
            no_classes_before: Some("8am".into()),
            no_classes_after: Some("21:00".into()),
            days_off: vec![],
        }),
    })
    .unwrap();
    assert!(parsed["schedules"].is_array());
}

#[test]
fn a_plan_file_that_cannot_be_read_is_a_storage_error() {
    let dir = temp_dir("storage");
    let path = dir.join("user.json");
    fs::write(&path, b"{not json").unwrap();
    let error = tools::list_plans(&dir).unwrap_err();
    assert_eq!(error.error, "storage_error");
    assert!(error.message.contains(path.to_str().unwrap()));
    assert!(error.message.contains("malformed JSON"));
    assert!(!error.message.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"{not json");

    fs::write(&path, br#"{"version": 9, "degreePlans": []}"#).unwrap();
    let newer = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "Should not replace".into(),
            terms: None,
        },
    )
    .unwrap_err();
    assert_eq!(newer.error, "storage_error");
    assert!(newer.message.contains("unknown newer version"));
    assert_eq!(
        fs::read(&path).unwrap(),
        br#"{"version": 9, "degreePlans": []}"#
    );
    let _ = fs::remove_dir_all(&dir);
}

fn spring_only_course() -> CourseKey {
    let catalogs = term_organize_mcp::catalogs::catalogs();
    let spring = catalogs
        .iter()
        .find(|index| index.catalog.term.code == "202730")
        .unwrap();
    let fall = catalogs
        .iter()
        .find(|index| index.catalog.term.code == "202710")
        .unwrap();
    spring
        .catalog
        .courses
        .iter()
        .find(|course| {
            course.prereqs.is_none()
                && !spring.sections_for(&course.key()).is_empty()
                && fall.sections_for(&course.key()).is_empty()
        })
        .map(|course| course.key())
        .expect("a course offered in Spring 2027 and not Fall 2026")
}

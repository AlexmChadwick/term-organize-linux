use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use term_organize_mcp::server::TermOrganize;
use term_organize_mcp::tools;
use term_organize_mcp::{resolve_mcp_data_dir, INSTRUCTIONS};
use termorganize_core::email::is_northeastern_address;
use termorganize_core::model::{
    CourseKey, DegreePlanCourse, PlanCourse, PlanSeason, ScheduleFilters, SchedulePlan,
};
use termorganize_core::planfile;
use termorganize_core::store;
use termorganize_core::UserData;
use uuid::Uuid;

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
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::All,
    })
    .unwrap();
    assert_eq!(found["courses"][0]["subject"], "CS");
    assert_eq!(found["courses"][0]["number"], "2100");
    assert!(found["courses"][0]["title"].as_str().unwrap().len() > 3);
    assert!(found["courses"][0].get("credits").is_some());
    assert!(found["courses"][0]["cps_only"].is_null());

    let course = tools::get_course(fall_course()).unwrap();
    assert!(course["cps_only"].is_null());
    assert!(course["sections"][0]["cps_only"].is_null());
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
            credits: None,
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
            credits: None,
            expected_revision: removed["revision"].as_u64().unwrap(),
        },
    )
    .unwrap_err();
    assert_eq!(unknown.error, "not_found");
    assert!(!unknown.message.is_empty());
    assert!(unknown.to_value()["error"]["code"].as_str().unwrap() == "not_found");
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
        "set_credits",
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
    assert!(!names.contains(&"set_credits"));
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
    assert_eq!(numbers, ["4805", "2535", "2700", "4130", "2701"]);
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

#[test]
fn missing_revision_is_one_and_unknown_catalog_rows_are_not_found() {
    let dir = temp_dir("revision-one");
    let mut data = UserData::default();
    let mut plan = termorganize_core::model::DegreePlan {
        name: "From the app".into(),
        terms: vec![termorganize_core::model::DegreePlanTerm::new(
            PlanSeason::Spring,
            2027,
        )],
        ..termorganize_core::model::DegreePlan::default()
    };
    plan.terms[0].courses.push(DegreePlanCourse::catalog(
        CourseKey::parse("ZZ 9999").unwrap(),
        "Not a course",
        4.0,
    ));
    let id = data.add_degree_plan(plan);
    data.plan_revisions.clear();
    fs::write(dir.join("user.json"), serde_json::to_string(&data).unwrap()).unwrap();

    let listed = tools::list_plans(&dir).unwrap();
    assert_eq!(listed["plans"][0]["revision"], 1);
    let loaded = tools::get_plan(&dir, &id.to_string()).unwrap();
    assert_eq!(loaded["revision"], 1);
    assert!(loaded["validation"].get("unoffered").is_none());
    assert_eq!(loaded["validation"]["not_offered"][0]["subject"], "ZZ");
    assert_eq!(loaded["validation"]["not_offered"][0]["number"], "9999");
    assert_eq!(
        loaded["validation"]["not_offered"][0]["term"],
        "Spring 2027"
    );

    let renamed = tools::rename_plan(
        &dir,
        tools::RenameArgs {
            plan_id: id.to_string(),
            name: "Still the app plan".into(),
            expected_revision: 1,
        },
    )
    .unwrap();
    assert_eq!(renamed["revision"], 2);
    let stale = tools::rename_plan(
        &dir,
        tools::RenameArgs {
            plan_id: id.to_string(),
            name: "Too old".into(),
            expected_revision: 1,
        },
    )
    .unwrap_err();
    assert_eq!(stale.error, "revision_conflict");

    let unknown_term = tools::get_course(tools::CourseArgs {
        term: "Fall 2024".into(),
        subject: "CS".into(),
        number: "3000".into(),
    })
    .unwrap_err();
    assert_eq!(unknown_term.error, "not_found");
    assert!(!unknown_term.message.is_empty());
    let malformed = tools::get_course(tools::CourseArgs {
        term: "not a term".into(),
        subject: "CS".into(),
        number: "3000".into(),
    })
    .unwrap_err();
    assert_eq!(malformed.error, "invalid_argument");

    let course = tools::get_course(tools::CourseArgs {
        term: "Spring 2027".into(),
        subject: "CS".into(),
        number: "3000".into(),
    })
    .unwrap();
    assert!(!course["sections"].as_array().unwrap().is_empty());
    let wrong_crn = tools::set_section(
        &dir,
        tools::SetSectionArgs {
            plan_id: id.to_string(),
            term: "Spring 2027".into(),
            subject: "CS".into(),
            number: "3000".into(),
            crn: "00000".into(),
            expected_revision: 2,
        },
    )
    .unwrap_err();
    assert_eq!(wrong_crn.error, "not_found");
    assert!(wrong_crn.message.contains("00000"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn distinct_crns_stay_separate_and_a_move_keeps_the_pin() {
    let catalogs = term_organize_mcp::catalogs::catalogs();
    let fall = catalogs
        .iter()
        .find(|index| index.catalog.term.code == "202710")
        .unwrap();
    let spring = catalogs
        .iter()
        .find(|index| index.catalog.term.code == "202730")
        .unwrap();
    let mut picked = None;
    for course in &fall.catalog.courses {
        if spring.sections_for(&course.key()).is_empty() {
            continue;
        }
        let sections = fall.sections_for(&course.key());
        if sections.len() < 3 {
            continue;
        }
        for left in 0..sections.len() {
            for right in (left + 1)..sections.len() {
                if sections[left].conflicts_with(&sections[right]) {
                    continue;
                }
                let third = sections
                    .iter()
                    .find(|section| {
                        section.crn != sections[left].crn && section.crn != sections[right].crn
                    })
                    .map(|section| section.crn.clone());
                picked = Some((
                    course.subject.clone(),
                    course.number.clone(),
                    sections[left].crn.clone(),
                    sections[right].crn.clone(),
                    third,
                ));
                break;
            }
            if picked.is_some() {
                break;
            }
        }
        if picked.is_some() {
            break;
        }
    }
    let (subject, number, first, second, third) =
        picked.expect("a course in both terms with three sections");
    let third = third.expect("third section");

    let duplicate = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: None,
            },
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: None,
            },
        ],
        constraints: None,
    })
    .unwrap_err();
    assert_eq!(duplicate.error, "invalid_argument");

    let repeated = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: Some(first.clone()),
            },
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: Some(first.clone()),
            },
        ],
        constraints: None,
    })
    .unwrap_err();
    assert_eq!(repeated.error, "invalid_argument");
    assert!(repeated.message.contains(&first));

    let suggested = tools::suggest_schedule(tools::SuggestArgs {
        term: "Fall 2026".into(),
        courses: vec![
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: Some(first.clone()),
            },
            tools::CourseRef {
                subject: subject.clone(),
                number: number.clone(),
                crn: Some(second.clone()),
            },
        ],
        constraints: None,
    })
    .unwrap();
    let schedules = suggested["schedules"].as_array().unwrap();
    assert!(!schedules.is_empty());
    for schedule in schedules {
        let crns: Vec<&str> = schedule["crns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|crn| crn.as_str().unwrap())
            .collect();
        assert_eq!(crns.len(), 2);
        assert!(crns.contains(&first.as_str()));
        assert!(crns.contains(&second.as_str()));
    }

    let dir = temp_dir("distinct-crn");
    let created = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "Topics".into(),
            terms: Some(vec!["Fall 2026".into(), "Spring 2027".into()]),
        },
    )
    .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let added = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: Some(first.clone()),
            credits: None,
            expected_revision: 1,
        },
    )
    .unwrap();
    let both = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: Some(second.clone()),
            credits: None,
            expected_revision: added["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(both["terms"][0]["courses"].as_array().unwrap().len(), 2);
    assert_eq!(both["terms"][0]["courses"][0]["crn"], first);
    assert_eq!(both["terms"][0]["courses"][1]["crn"], second);
    assert!(both["validation"]["duplicates"]
        .as_array()
        .unwrap()
        .is_empty());

    let ambiguous = tools::set_section(
        &dir,
        tools::SetSectionArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: third,
            expected_revision: both["revision"].as_u64().unwrap(),
        },
    )
    .unwrap_err();
    assert_eq!(ambiguous.error, "invalid_argument");
    assert!(ambiguous.message.contains("more than once"));

    let moved = tools::move_course(
        &dir,
        tools::MoveCourseArgs {
            plan_id: id.clone(),
            subject: subject.clone(),
            number: number.clone(),
            from_term: "Fall 2026".into(),
            to_term: "Spring 2027".into(),
            expected_revision: both["revision"].as_u64().unwrap(),
        },
    )
    .unwrap_err();
    assert_eq!(moved.error, "invalid_argument");
    assert!(moved.message.contains("more than once"));

    let same_section = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: Some(first.clone()),
            credits: None,
            expected_revision: both["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        same_section["terms"][0]["courses"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let notice = format!("{subject} {number} appears more than once. Each copy still counts.");
    assert!(same_section["validation"]["duplicates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str() == Some(notice.as_str())));

    let unpinned = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: None,
            credits: None,
            expected_revision: same_section["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(unpinned["terms"][0]["courses"].as_array().unwrap().len(), 4);

    let single = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "One section".into(),
            terms: Some(vec!["Fall 2026".into(), "Spring 2027".into()]),
        },
    )
    .unwrap();
    let single_id = single["id"].as_str().unwrap().to_string();
    let pinned = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: single_id.clone(),
            term: "Fall 2026".into(),
            subject: subject.clone(),
            number: number.clone(),
            crn: Some(first.clone()),
            credits: None,
            expected_revision: 1,
        },
    )
    .unwrap();
    let kept = tools::move_course(
        &dir,
        tools::MoveCourseArgs {
            plan_id: single_id.clone(),
            subject: subject.clone(),
            number: number.clone(),
            from_term: "Fall 2026".into(),
            to_term: "Spring 2027".into(),
            expected_revision: pinned["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(kept["terms"][1]["courses"][0]["crn"], first);
    let again = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: single_id.clone(),
            term: "Fall 2026".into(),
            subject,
            number,
            crn: None,
            credits: None,
            expected_revision: kept["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    let joined = tools::move_course(
        &dir,
        tools::MoveCourseArgs {
            plan_id: single_id.clone(),
            subject: again["terms"][0]["courses"][0]["subject"]
                .as_str()
                .unwrap()
                .to_string(),
            number: again["terms"][0]["courses"][0]["number"]
                .as_str()
                .unwrap()
                .to_string(),
            from_term: "Fall 2026".into(),
            to_term: "Spring 2027".into(),
            expected_revision: again["revision"].as_u64().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(joined["terms"][1]["courses"].as_array().unwrap().len(), 2);
    assert_eq!(joined["terms"][1]["courses"][0]["crn"], first);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn search_tool_requisite_filters_keep_loaded_courses_with_none() {
    let dir = temp_dir("req-filter");
    let server = TermOrganize::new(dir.clone(), false);
    let tools = server.tool_definitions();
    let tool = tools
        .iter()
        .find(|tool| tool.name.as_ref() == "search_courses")
        .unwrap();
    let props = tool
        .input_schema
        .get("properties")
        .and_then(|value| value.as_object())
        .unwrap();
    assert!(props.contains_key("no_prerequisites"));
    assert!(props.contains_key("no_corequisites"));
    assert!(props.contains_key("level"));
    let description = tool.description.as_deref().unwrap();
    assert!(description.contains("no_prerequisites"));
    assert!(description.contains("no_corequisites"));
    assert!(description.contains("undergrad"));

    let omitted: tools::SearchArgs =
        serde_json::from_str(r#"{"term":"Fall 2026","subject":"CS","limit":5}"#).unwrap();
    assert!(!omitted.no_prerequisites);
    assert!(!omitted.no_corequisites);
    assert_eq!(omitted.level, tools::SearchLevel::All);

    let index = term_organize_mcp::catalogs::catalogs()
        .iter()
        .find(|index| index.catalog.term.code == "202710")
        .unwrap();
    let courses = &index.catalog.courses;
    let open = courses
        .iter()
        .find(|course| course.has_no_prerequisites() && course.has_no_corequisites())
        .expect("loaded course with neither requisite");
    let needs = courses
        .iter()
        .find(|course| course.details_loaded && !course.has_no_prerequisites())
        .expect("loaded course with prerequisites");
    let needs_coreq = courses
        .iter()
        .find(|course| course.details_loaded && !course.has_no_corequisites())
        .expect("loaded course with corequisites");

    let listed = |course: &termorganize_core::model::Course, prereq: bool, coreq: bool| {
        let found = tools::search_courses(tools::SearchArgs {
            term: "202710".into(),
            query: None,
            subject: Some(course.subject.clone()),
            number: Some(course.number.clone()),
            attribute: None,
            limit: Some(25),
            no_prerequisites: prereq,
            no_corequisites: coreq,
            level: tools::SearchLevel::All,
        })
        .unwrap();
        found["courses"].as_array().unwrap().iter().any(|row| {
            row["subject"].as_str() == Some(course.subject.as_str())
                && row["number"].as_str() == Some(course.number.as_str())
        })
    };

    assert!(listed(open, true, true), "{}", open.code());
    assert!(!listed(needs, true, false), "{}", needs.code());
    assert!(!listed(needs_coreq, false, true), "{}", needs_coreq.code());
    if let Some(unknown) = courses.iter().find(|course| {
        !course.details_loaded
            && !courses.iter().any(|other| {
                other.details_loaded
                    && other.subject == course.subject
                    && other.number == course.number
                    && other.has_no_prerequisites()
                    && other.has_no_corequisites()
            })
    }) {
        assert!(!listed(unknown, true, true), "{}", unknown.code());
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn search_level_splits_undergrad_and_grad() {
    let under = tools::search_courses(tools::SearchArgs {
        term: "Fall 2026".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("2100".into()),
        attribute: None,
        limit: Some(5),
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::Undergrad,
    })
    .unwrap();
    assert_eq!(under["courses"][0]["number"], "2100");
    let hidden = tools::search_courses(tools::SearchArgs {
        term: "Fall 2026".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("2100".into()),
        attribute: None,
        limit: Some(5),
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::Grad,
    })
    .unwrap();
    assert!(hidden["courses"].as_array().unwrap().is_empty());
    let grad = tools::search_courses(tools::SearchArgs {
        term: "Fall 2026".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("5200".into()),
        attribute: None,
        limit: Some(5),
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::Grad,
    })
    .unwrap();
    assert_eq!(grad["courses"][0]["number"], "5200");
    let under_hides_grad = tools::search_courses(tools::SearchArgs {
        term: "Fall 2026".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("5200".into()),
        attribute: None,
        limit: Some(5),
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::Undergrad,
    })
    .unwrap();
    assert!(under_hides_grad["courses"].as_array().unwrap().is_empty());
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

#[test]
fn variable_credit_add_and_set_credits_follow_the_row_rules() {
    let found = tools::search_courses(tools::SearchArgs {
        term: "Fall 2026".into(),
        query: None,
        subject: Some("CS".into()),
        number: Some("8982".into()),
        attribute: None,
        limit: Some(5),
        no_prerequisites: false,
        no_corequisites: false,
        level: tools::SearchLevel::All,
    })
    .unwrap();
    let course = &found["courses"][0];
    assert_eq!(course["credit_low"].as_f64().unwrap(), 1.0);
    assert_eq!(course["credit_high"].as_f64().unwrap(), 8.0);
    assert_eq!(course["chosen_credits"].as_f64().unwrap(), 4.0);
    assert!(course["credits"].as_str().unwrap().contains('1'));
    assert!(course["credits"].as_str().unwrap().contains('8'));

    let detail = tools::get_course(tools::CourseArgs {
        term: "Fall 2026".into(),
        subject: "CS".into(),
        number: "8982".into(),
    })
    .unwrap();
    let crns: Vec<String> = detail["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|section| section["crn"].as_str().map(str::to_string))
        .collect();
    assert!(!crns.is_empty());

    let dir = temp_dir("credits");
    let created = tools::create_plan(
        &dir,
        tools::CreatePlanArgs {
            name: "Variable credits".into(),
            terms: Some(vec!["Fall 2026".into()]),
        },
    )
    .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let added = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: Some(crns[0].clone()),
            credits: None,
            expected_revision: 1,
        },
    )
    .unwrap();
    assert_eq!(
        added["terms"][0]["courses"][0]["credits"].as_f64().unwrap(),
        4.0
    );
    assert_eq!(
        added["terms"][0]["courses"][0]["credit_low"]
            .as_f64()
            .unwrap(),
        1.0
    );
    assert_eq!(
        added["terms"][0]["courses"][0]["credit_high"]
            .as_f64()
            .unwrap(),
        8.0
    );
    assert_eq!(added["revision"].as_u64().unwrap(), 2);

    let too_high = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: None,
            credits: Some(9.0),
            expected_revision: 2,
        },
    )
    .unwrap_err();
    assert_eq!(too_high.error, "invalid_argument");
    assert!(too_high.message.contains("whole number from 1 to 8"));

    let fractional = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: None,
            credits: Some(4.5),
            expected_revision: 2,
        },
    )
    .unwrap_err();
    assert_eq!(fractional.error, "invalid_argument");
    assert!(fractional.message.contains("whole number from 1 to 8"));

    let fixed = tools::set_credits(
        &dir,
        tools::SetCreditsArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "2100".into(),
            crn: None,
            credits: 4.0,
            expected_revision: 2,
        },
    )
    .unwrap_err();
    assert_eq!(fixed.error, "invalid_argument", "{}", fixed.message);
    assert!(fixed.message.contains("fixed credit"), "{}", fixed.message);

    for value in [0.0, 9.0] {
        let rejected = tools::set_credits(
            &dir,
            tools::SetCreditsArgs {
                plan_id: id.clone(),
                term: "Fall 2026".into(),
                subject: "CS".into(),
                number: "8982".into(),
                crn: None,
                credits: value,
                expected_revision: 2,
            },
        )
        .unwrap_err();
        assert_eq!(rejected.error, "invalid_argument");
        assert!(rejected.message.contains("whole number from 1 to 8"));
    }

    let second = tools::add_course(
        &dir,
        tools::AddCourseArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: crns.get(1).cloned(),
            credits: Some(2.0),
            expected_revision: 2,
        },
    )
    .unwrap();
    assert_eq!(second["revision"].as_u64().unwrap(), 3);
    let rows = second["terms"][0]["courses"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["credits"].as_f64().unwrap(), 4.0);
    assert_eq!(rows[1]["credits"].as_f64().unwrap(), 2.0);

    let ambiguous = tools::set_credits(
        &dir,
        tools::SetCreditsArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: None,
            credits: 5.0,
            expected_revision: 3,
        },
    )
    .unwrap_err();
    assert_eq!(ambiguous.error, "invalid_argument");
    assert!(ambiguous.message.contains("more than once"));

    let mut data = store::load_user(&dir);
    let row_id = data.degree_plans[0].terms[0].courses[0].id;
    data.plans.push(SchedulePlan {
        id: Uuid::new_v4(),
        name: "Fall plan".into(),
        term_code: "202710".into(),
        courses: vec![PlanCourse {
            key: CourseKey::parse("CS 8982").unwrap(),
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index: 0,
            pinned_crn: Some(crns[0].clone()),
            degree_row_id: Some(row_id),
            credits: Some(4.0),
        }],
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: true,
        created_at: 0.0,
        updated_at: 0.0,
    });
    store::save_user(&dir, &data).unwrap();

    let updated = tools::set_credits(
        &dir,
        tools::SetCreditsArgs {
            plan_id: id.clone(),
            term: "Fall 2026".into(),
            subject: "CS".into(),
            number: "8982".into(),
            crn: Some(crns[0].clone()),
            credits: 6.0,
            expected_revision: 3,
        },
    )
    .unwrap();
    let rows = updated["terms"][0]["courses"].as_array().unwrap();
    assert_eq!(rows[0]["credits"].as_f64().unwrap(), 6.0);
    assert_eq!(rows[0]["crn"].as_str().unwrap(), crns[0]);
    assert_eq!(rows[1]["credits"].as_f64().unwrap(), 2.0);
    let linked = store::load_user(&dir);
    assert_eq!(linked.plans[0].courses[0].credits, Some(6.0));
    assert!((linked.degree_plans[0].terms[0].courses[0].credits - 6.0).abs() < 1e-9);
    assert!((linked.degree_plans[0].terms[0].courses[1].credits - 2.0).abs() < 1e-9);
    let _ = fs::remove_dir_all(&dir);
}

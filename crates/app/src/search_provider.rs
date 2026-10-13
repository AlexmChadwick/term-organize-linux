//! GNOME Shell search provider. Replaces the Mac app's Spotlight importer.
//!
//! Shell looks up `org.termorganize.TermOrganize.SearchProvider` on the session bus
//! and calls `org.gnome.Shell.SearchProvider2`.

use crate::load_index;
use gtk::{gio, glib};
use glib::prelude::*;
use std::rc::Rc;
use termorganize_core::index::CatalogIndex;
use termorganize_core::search::{self, SearchQuery};

const XML: &str = r#"
<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet">
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetSubsearchResultSet">
      <arg type="as" name="previous_results" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetResultMetas">
      <arg type="as" name="identifiers" direction="in"/>
      <arg type="aa{sv}" name="metas" direction="out"/>
    </method>
    <method name="ActivateResult">
      <arg type="s" name="identifier" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="LaunchSearch">
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
  </interface>
</node>
"#;

pub fn serve() -> i32 {
    let Some(index) = load_index() else {
        eprintln!("termorganize: no catalog for the search provider");
        return 1;
    };
    let index = Rc::new(index);
    let node = gio::DBusNodeInfo::for_xml(XML).expect("search provider interface");
    let node = Box::leak(Box::new(node));
    let Some(iface) = node.lookup_interface("org.gnome.Shell.SearchProvider2") else {
        eprintln!("termorganize: search provider interface missing");
        return 1;
    };
    let iface = Box::leak(Box::new(iface));
    let owned = index.clone();
    gio::bus_own_name(
        gio::BusType::Session,
        "org.termorganize.TermOrganize.SearchProvider",
        gio::BusNameOwnerFlags::NONE,
        move |connection, _name| {
            let index = owned.clone();
            if let Err(error) = connection
                .register_object("/org/termorganize/SearchProvider", iface)
                .method_call(move |_connection, _sender, _path, _interface, method, params, invocation| {
                    dispatch(&index, method, &params, invocation);
                })
                .build()
            {
                eprintln!("termorganize: could not export the search provider: {error}");
            }
        },
        |_connection, _name| {},
        |_connection, _name| {},
    );
    glib::MainLoop::new(None, false).run();
    0
}

fn dispatch(index: &CatalogIndex, method: &str, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
    match method {
        "GetInitialResultSet" => {
            let terms = string_list(&params.child_value(0));
            reply_strings(invocation, &result_ids(index, &terms.join(" ")));
        }
        "GetSubsearchResultSet" => {
            let terms = string_list(&params.child_value(1));
            reply_strings(invocation, &result_ids(index, &terms.join(" ")));
        }
        "GetResultMetas" => {
            let ids = string_list(&params.child_value(0));
            let metas: Vec<glib::Variant> = ids.iter().map(|id| meta(index, id)).collect();
            let array = glib::Variant::array_from_iter::<glib::Variant>(metas);
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([array])));
        }
        "ActivateResult" => {
            let id = params.child_value(0).str().unwrap_or("").to_string();
            launch(&format!("termorganize://search?q={}", id.replace(' ', "+")));
            invocation.return_value(None);
        }
        "LaunchSearch" => {
            let terms = string_list(&params.child_value(0));
            launch(&format!("termorganize://search?q={}", terms.join("+")));
            invocation.return_value(None);
        }
        _ => invocation.return_dbus_error("org.termorganize.Error", "Unknown method"),
    }
}

fn result_ids(index: &CatalogIndex, text: &str) -> Vec<String> {
    let response = search::search(&SearchQuery { text: text.into(), ..SearchQuery::default() }, index);
    let mut ids = Vec::new();
    if let Some(section) = &response.crn_match {
        ids.push(format!("CRN {}", section.crn));
    }
    for hit in response.results.iter().take(8) {
        ids.push(hit.course.code());
    }
    ids
}

fn meta(index: &CatalogIndex, id: &str) -> glib::Variant {
    let description = if let Some(rest) = id.strip_prefix("CRN ") {
        format!("Section {rest}")
    } else if let Some(key) = termorganize_core::model::CourseKey::parse(id) {
        index.course(&key).map(|c| c.title.clone()).unwrap_or_else(|| "Course".into())
    } else {
        "Term Organize".into()
    };
    let dict = glib::VariantDict::new(None);
    dict.insert("id", id);
    dict.insert("name", id);
    dict.insert("description", &description);
    dict.end()
}

fn reply_strings(invocation: gio::DBusMethodInvocation, ids: &[String]) {
    let array = glib::Variant::array_from_iter::<glib::Variant>(ids.iter().map(|id| id.to_variant()));
    invocation.return_value(Some(&glib::Variant::tuple_from_iter([array])));
}

fn string_list(value: &glib::Variant) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..value.n_children() {
        if let Some(text) = value.child_value(i).str() {
            out.push(text.to_string());
        }
    }
    out
}

fn launch(url: &str) {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("termorganize"));
    let _ = std::process::Command::new(exe).arg(url).spawn();
}

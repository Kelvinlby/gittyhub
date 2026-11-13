//! Generic form dialogs: a list of fields in an `AdwAlertDialog`.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

pub enum Field {
    Entry {
        label: String,
        value: String,
        required: bool,
    },
    Text {
        label: String,
        value: String,
    },
    Switch {
        label: String,
        subtitle: String,
        value: bool,
    },
    Choice {
        label: String,
        options: Vec<String>,
        selected: usize,
    },
}

impl Field {
    pub fn entry(label: &str, value: &str) -> Field {
        Field::Entry {
            label: label.into(),
            value: value.into(),
            required: false,
        }
    }
    pub fn required(label: &str, value: &str) -> Field {
        Field::Entry {
            label: label.into(),
            value: value.into(),
            required: true,
        }
    }
    pub fn text(label: &str, value: &str) -> Field {
        Field::Text {
            label: label.into(),
            value: value.into(),
        }
    }
    pub fn switch(label: &str, subtitle: &str, value: bool) -> Field {
        Field::Switch {
            label: label.into(),
            subtitle: subtitle.into(),
            value,
        }
    }
    pub fn choice<S: AsRef<str>>(label: &str, options: &[S], selected: usize) -> Field {
        Field::Choice {
            label: label.into(),
            options: options.iter().map(|s| s.as_ref().to_owned()).collect(),
            selected,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    Bool(bool),
    Index(usize),
}

impl Value {
    pub fn text(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Index(i) => i.to_string(),
        }
    }
    pub fn bool(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    pub fn index(&self) -> usize {
        match self {
            Value::Index(i) => *i,
            _ => 0,
        }
    }
}

enum Getter {
    Entry(adw::EntryRow),
    Text(gtk::TextBuffer),
    Switch(adw::SwitchRow),
    Choice(adw::ComboRow),
}

/// Show a form; resolves to the field values, or `None` when cancelled.
pub async fn run(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    submit: &str,
    fields: Vec<Field>,
) -> Option<Vec<Value>> {
    let dialog = adw::AlertDialog::new(Some(heading), (!body.is_empty()).then_some(body));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", submit)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let mut list: Option<gtk::ListBox> = None;
    let mut getters = Vec::new();
    let mut required = Vec::new();
    let mut has_text = false;
    let ensure_list = |list: &mut Option<gtk::ListBox>, container: &gtk::Box| {
        list.get_or_insert_with(|| {
            let l = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            container.append(&l);
            l
        })
        .clone()
    };
    for f in fields {
        match f {
            Field::Entry {
                label,
                value,
                required: req,
            } => {
                let row = adw::EntryRow::builder().title(label).text(value).build();
                row.set_activates_default(true);
                ensure_list(&mut list, &container).append(&row);
                if req {
                    required.push(row.clone());
                }
                getters.push(Getter::Entry(row));
            }
            Field::Switch {
                label,
                subtitle,
                value,
            } => {
                let row = adw::SwitchRow::builder()
                    .title(label)
                    .subtitle(subtitle)
                    .active(value)
                    .build();
                ensure_list(&mut list, &container).append(&row);
                getters.push(Getter::Switch(row));
            }
            Field::Choice {
                label,
                options,
                selected,
            } => {
                let strs: Vec<&str> = options.iter().map(String::as_str).collect();
                let row = adw::ComboRow::builder()
                    .title(label)
                    .model(&gtk::StringList::new(&strs))
                    .selected(selected as u32)
                    .enable_search(options.len() > 8)
                    .build();
                ensure_list(&mut list, &container).append(&row);
                getters.push(Getter::Choice(row));
            }
            Field::Text { label, value } => {
                has_text = true;
                list = None;
                let title = gtk::Label::builder()
                    .label(label)
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build();
                let buffer = gtk::TextBuffer::new(None);
                buffer.set_text(&value);
                let view = gtk::TextView::builder()
                    .buffer(&buffer)
                    .wrap_mode(gtk::WrapMode::WordChar)
                    .top_margin(8)
                    .bottom_margin(8)
                    .left_margin(8)
                    .right_margin(8)
                    .accepts_tab(false)
                    .build();
                let scroller = gtk::ScrolledWindow::builder()
                    .child(&view)
                    .min_content_height(140)
                    .max_content_height(320)
                    .propagate_natural_height(true)
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .css_classes(["text-area"])
                    .overflow(gtk::Overflow::Hidden)
                    .build();
                container.append(&title);
                container.append(&scroller);
                getters.push(Getter::Text(buffer));
            }
        }
    }
    dialog.set_prefer_wide_layout(has_text);
    dialog.set_extra_child(Some(&container));

    let required = Rc::new(required);
    let update = {
        let dialog = dialog.clone();
        let required = required.clone();
        move || {
            let ok = required.iter().all(|r| !r.text().trim().is_empty());
            dialog.set_response_enabled("ok", ok);
        }
    };
    update();
    for r in required.iter() {
        let u = update.clone();
        r.connect_changed(move |_| u());
    }
    if let Some(Getter::Entry(first)) = getters.first() {
        let first = first.clone();
        glib::idle_add_local_once(move || {
            first.grab_focus();
        });
    }

    if dialog.choose_future(Some(parent)).await != "ok" {
        return None;
    }
    Some(
        getters
            .iter()
            .map(|g| match g {
                Getter::Entry(r) => Value::Text(r.text().trim().to_owned()),
                Getter::Text(b) => {
                    Value::Text(b.text(&b.start_iter(), &b.end_iter(), false).to_string())
                }
                Getter::Switch(r) => Value::Bool(r.is_active()),
                Getter::Choice(r) => Value::Index(r.selected() as usize),
            })
            .collect(),
    )
}

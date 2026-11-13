//! IDE-style file tree: folders and files with status letters, include checkboxes and
//! right-click menus. Built on `GtkListView` + `GtkTreeListModel`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

type PathFn = Rc<dyn Fn(String)>;
type MenuFn = Rc<dyn Fn(&gtk::Widget, String, bool, f64, f64)>;
type ToggleFn = Rc<dyn Fn()>;
/// Bound checkboxes and the path each shows, for re-syncing after bulk changes.
type Checks = Rc<RefCell<Vec<(glib::WeakRef<gtk::CheckButton>, String)>>>;

#[derive(Default)]
struct Data {
    /// Directory path ("" for root, else ending in '/') → sorted children paths.
    children: HashMap<String, Vec<String>>,
    status: HashMap<String, char>,
    checkable: bool,
}

pub struct FileTree {
    pub widget: gtk::ScrolledWindow,
    view: gtk::ListView,
    selection: gtk::SingleSelection,
    data: Rc<RefCell<Data>>,
    /// Files left out of the next commit.
    pub excluded: Rc<RefCell<HashSet<String>>>,
    checks: Checks,
    binding: Rc<Cell<bool>>,
    on_select: Rc<RefCell<Option<PathFn>>>,
    on_menu: Rc<RefCell<Option<MenuFn>>>,
    on_toggle: Rc<RefCell<Option<ToggleFn>>>,
}

fn status_class(c: char) -> &'static str {
    match c {
        'A' | '?' => "added",
        'D' => "deleted",
        'R' | 'C' => "renamed",
        'U' => "conflict",
        _ => "modified",
    }
}

impl Data {
    /// Changed files at or under `dir` (only changed files can be included/excluded).
    fn files_under(&self, dir: &str) -> Vec<String> {
        self.status
            .keys()
            .filter(|p| p.starts_with(dir))
            .cloned()
            .collect()
    }
}

impl FileTree {
    pub fn new() -> Rc<Self> {
        let data = Rc::new(RefCell::new(Data::default()));
        let excluded: Rc<RefCell<HashSet<String>>> = Rc::default();
        let checks: Checks = Rc::default();
        let binding = Rc::new(Cell::new(false));
        let on_select: Rc<RefCell<Option<PathFn>>> = Rc::default();
        let on_menu: Rc<RefCell<Option<MenuFn>>> = Rc::default();
        let on_toggle: Rc<RefCell<Option<ToggleFn>>> = Rc::default();

        let factory = gtk::SignalListItemFactory::new();
        {
            let (data, excluded, checks, binding, on_menu, on_toggle) = (
                data.clone(),
                excluded.clone(),
                checks.clone(),
                binding.clone(),
                on_menu.clone(),
                on_toggle.clone(),
            );
            factory.connect_setup(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let expander = gtk::TreeExpander::new();
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let check = gtk::CheckButton::builder()
                    .valign(gtk::Align::Center)
                    .build();
                let icon = gtk::Image::new();
                let name = gtk::Label::builder()
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .build();
                let status = gtk::Label::builder().css_classes(["status-letter"]).build();
                row.append(&check);
                row.append(&icon);
                row.append(&name);
                row.append(&status);
                expander.set_child(Some(&row));
                item.set_child(Some(&expander));

                let exp = expander.downgrade();
                let (data, excluded, checks, binding, on_toggle) = (
                    data.clone(),
                    excluded.clone(),
                    checks.clone(),
                    binding.clone(),
                    on_toggle.clone(),
                );
                check.connect_toggled(move |c| {
                    if binding.get() {
                        return;
                    }
                    let Some(exp) = exp.upgrade() else { return };
                    let path = exp.widget_name().to_string();
                    let files = if path.ends_with('/') {
                        data.borrow().files_under(&path)
                    } else {
                        vec![path]
                    };
                    {
                        let mut ex = excluded.borrow_mut();
                        for f in files {
                            if c.is_active() {
                                ex.remove(&f);
                            } else {
                                ex.insert(f);
                            }
                        }
                    }
                    sync_checks(&checks, &excluded, &data, &binding);
                    let cb = on_toggle.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                });

                let gesture = gtk::GestureClick::builder().button(3).build();
                let exp = expander.downgrade();
                let on_menu = on_menu.clone();
                gesture.connect_pressed(move |g, _, x, y| {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    let Some(exp) = exp.upgrade() else { return };
                    let path = exp.widget_name().to_string();
                    let cb = on_menu.borrow().clone();
                    if let Some(cb) = cb {
                        let dir = path.ends_with('/');
                        cb(exp.upcast_ref(), path, dir, x, y);
                    }
                });
                expander.add_controller(gesture);
            });
        }
        {
            let (data, excluded, checks, binding) = (
                data.clone(),
                excluded.clone(),
                checks.clone(),
                binding.clone(),
            );
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else {
                    return;
                };
                let Some(obj) = row.item().and_downcast::<gtk::StringObject>() else {
                    return;
                };
                let path = obj.string().to_string();
                let expander = item
                    .child()
                    .and_downcast::<gtk::TreeExpander>()
                    .expect("expander");
                expander.set_list_row(Some(&row));
                expander.set_widget_name(&path);
                let b = expander.child().and_downcast::<gtk::Box>().expect("box");
                let check = b
                    .first_child()
                    .and_downcast::<gtk::CheckButton>()
                    .expect("check");
                let icon = check
                    .next_sibling()
                    .and_downcast::<gtk::Image>()
                    .expect("icon");
                let name = icon
                    .next_sibling()
                    .and_downcast::<gtk::Label>()
                    .expect("name");
                let status = name
                    .next_sibling()
                    .and_downcast::<gtk::Label>()
                    .expect("status");
                let is_dir = path.ends_with('/');
                let display = path
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(&path)
                    .to_owned();
                name.set_label(&display);
                name.set_tooltip_text(Some(&path));
                icon.set_icon_name(Some(if is_dir {
                    "folder-symbolic"
                } else {
                    "text-x-generic-symbolic"
                }));
                let d = data.borrow();
                check.set_visible(
                    d.checkable
                        && if is_dir {
                            !d.files_under(&path).is_empty()
                        } else {
                            d.status.contains_key(&path)
                        },
                );
                let st = d.status.get(&path).copied();
                status.set_label(&st.map(String::from).unwrap_or_default());
                match st {
                    Some(c) => status.set_css_classes(&["status-letter", status_class(c)]),
                    None => status.set_css_classes(&["status-letter"]),
                }
                drop(d);
                checks
                    .borrow_mut()
                    .retain(|(w, _)| w.upgrade().is_some_and(|c| c != check));
                checks.borrow_mut().push((check.downgrade(), path));
                sync_one(&check, &expander.widget_name(), &excluded, &data, &binding);
            });
        }

        let root = gio::ListStore::new::<gtk::StringObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&gtk::TreeListModel::new(root, false, true, |_| None))
            .autoselect(false)
            .can_unselect(true)
            .build();
        let view = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .single_click_activate(true)
            .css_classes(["navigation-sidebar", "file-tree"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let tree = Rc::new(FileTree {
            widget,
            view,
            selection,
            data,
            excluded,
            checks,
            binding,
            on_select,
            on_menu,
            on_toggle,
        });
        let w = Rc::downgrade(&tree);
        tree.view.connect_activate(move |_, pos| {
            let Some(t) = w.upgrade() else { return };
            let Some(row) = t.selection.item(pos).and_downcast::<gtk::TreeListRow>() else {
                return;
            };
            let Some(obj) = row.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let path = obj.string().to_string();
            if path.ends_with('/') {
                row.set_expanded(!row.is_expanded());
            } else {
                t.selection.set_selected(pos);
                let cb = t.on_select.borrow().clone();
                if let Some(cb) = cb {
                    cb(path);
                }
            }
        });
        tree
    }

    pub fn connect_select(&self, f: impl Fn(String) + 'static) {
        *self.on_select.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_menu(&self, f: impl Fn(&gtk::Widget, String, bool, f64, f64) + 'static) {
        *self.on_menu.borrow_mut() = Some(Rc::new(f));
    }

    pub fn connect_toggled(&self, f: impl Fn() + 'static) {
        *self.on_toggle.borrow_mut() = Some(Rc::new(f));
    }

    /// Replace the tree. `files` are (path, status letter); `expand` opens every folder.
    pub fn set_files(&self, files: &[(String, Option<char>)], checkable: bool, expand: bool) {
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut status = HashMap::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (path, st) in files {
            if let Some(s) = st {
                status.insert(path.clone(), *s);
            }
            let mut parent = String::new();
            let parts: Vec<&str> = path.split('/').collect();
            for (i, part) in parts.iter().enumerate() {
                let full = if i + 1 == parts.len() {
                    path.clone()
                } else {
                    format!("{parent}{part}/")
                };
                if seen.insert(full.clone()) {
                    children
                        .entry(parent.clone())
                        .or_default()
                        .push(full.clone());
                }
                parent = full;
            }
        }
        for v in children.values_mut() {
            v.sort_by(|a, b| {
                b.ends_with('/')
                    .cmp(&a.ends_with('/'))
                    .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
            });
        }
        let root_items = children.get("").cloned().unwrap_or_default();
        *self.data.borrow_mut() = Data {
            children,
            status,
            checkable,
        };
        let root = gio::ListStore::new::<gtk::StringObject>();
        for p in root_items {
            root.append(&gtk::StringObject::new(&p));
        }
        let data = self.data.clone();
        let model = gtk::TreeListModel::new(root, false, expand, move |item| {
            let path = item.downcast_ref::<gtk::StringObject>()?.string();
            if !path.ends_with('/') {
                return None;
            }
            let store = gio::ListStore::new::<gtk::StringObject>();
            for c in data
                .borrow()
                .children
                .get(path.as_str())
                .into_iter()
                .flatten()
            {
                store.append(&gtk::StringObject::new(c));
            }
            Some(store.upcast())
        });
        self.selection.set_model(Some(&model));
        self.selection.set_selected(gtk::INVALID_LIST_POSITION);
    }

    /// Select (and reveal) `path` if it is in the tree.
    pub fn select_path(&self, path: &str) {
        let Some(model) = self.selection.model().and_downcast::<gtk::TreeListModel>() else {
            return;
        };
        // Expand ancestors so the row exists.
        let mut prefix = String::new();
        for part in path
            .split('/')
            .collect::<Vec<_>>()
            .split_last()
            .map(|x| x.1)
            .unwrap_or(&[])
        {
            prefix.push_str(part);
            prefix.push('/');
            for i in 0..model.n_items() {
                if let Some(r) = model.row(i)
                    && r.item()
                        .and_downcast::<gtk::StringObject>()
                        .is_some_and(|o| o.string() == prefix)
                {
                    r.set_expanded(true);
                }
            }
        }
        for i in 0..model.n_items() {
            if let Some(r) = model.row(i)
                && r.item()
                    .and_downcast::<gtk::StringObject>()
                    .is_some_and(|o| o.string() == path)
            {
                self.selection.set_selected(i);
                self.view.scroll_to(i, gtk::ListScrollFlags::NONE, None);
                return;
            }
        }
    }

    pub fn has_file(&self, path: &str) -> bool {
        let d = self.data.borrow();
        d.status.contains_key(path) || d.children.values().flatten().any(|p| p == path)
    }

    /// Re-sync visible checkboxes after `excluded` changed externally.
    pub fn sync(&self) {
        sync_checks(&self.checks, &self.excluded, &self.data, &self.binding);
    }
}

fn sync_one(
    check: &gtk::CheckButton,
    path: &str,
    excluded: &Rc<RefCell<HashSet<String>>>,
    data: &Rc<RefCell<Data>>,
    binding: &Rc<Cell<bool>>,
) {
    binding.set(true);
    let ex = excluded.borrow();
    if path.ends_with('/') {
        let files = data.borrow().files_under(path);
        let n_ex = files.iter().filter(|f| ex.contains(*f)).count();
        check.set_active(n_ex < files.len());
        check.set_inconsistent(n_ex > 0 && n_ex < files.len());
    } else {
        check.set_inconsistent(false);
        check.set_active(!ex.contains(path));
    }
    binding.set(false);
}

fn sync_checks(
    checks: &Checks,
    excluded: &Rc<RefCell<HashSet<String>>>,
    data: &Rc<RefCell<Data>>,
    binding: &Rc<Cell<bool>>,
) {
    checks.borrow_mut().retain(|(w, _)| w.upgrade().is_some());
    let list: Vec<_> = checks.borrow().clone();
    for (w, path) in list {
        if let Some(c) = w.upgrade() {
            sync_one(&c, &path, excluded, data, binding);
        }
    }
}

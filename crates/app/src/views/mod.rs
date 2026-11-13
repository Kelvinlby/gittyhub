pub mod actions;
pub mod branches;
pub mod code;
pub mod deployments;
pub mod github;
pub mod issues;
pub mod overview;
pub mod packages;
pub mod pulls;
pub mod releases;
pub mod remotes;
pub mod stashes;
pub mod tags;

use std::rc::Rc;

use adw::prelude::*;

use crate::repo::View;

/// Several views side by side (stacked when the window is narrow).
struct Columns {
    root: adw::BreakpointBin,
    views: Vec<Rc<dyn View>>,
}

pub fn columns(views: Vec<Rc<dyn View>>) -> Rc<dyn View> {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    for (i, v) in views.iter().enumerate() {
        if i > 0 {
            row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        }
        let w = v.widget();
        w.set_hexpand(true);
        w.set_vexpand(true);
        row.append(&w);
    }
    let root = adw::BreakpointBin::builder()
        .width_request(360)
        .height_request(300)
        .child(&row)
        .build();
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        900.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(
        &row,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    root.add_breakpoint(bp);
    Rc::new(Columns { root, views })
}

impl View for Columns {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn refresh(self: Rc<Self>) {
        for v in &self.views {
            v.clone().refresh();
        }
    }
}

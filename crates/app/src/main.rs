mod diff;
mod editor;
mod form;
mod markdown;
mod open;
mod prefs;
mod repo;
mod settings;
mod tree;
mod util;
mod views;
mod window;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "io.github.Kelvinlby.GittyHub";

fn main() -> glib::ExitCode {
    gio::resources_register_include!("gittyhub.gresource").expect("bundled resources");
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.add_main_option(
        "new-window",
        glib::Char::from(b'n'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open a new window",
        None,
    );
    app.connect_handle_local_options(|app, opts| {
        if opts.contains("new-window") {
            if app.register(gio::Cancellable::NONE).is_ok() && app.is_remote() {
                // Already running: ask that instance for a window and exit.
                app.activate_action("new-window", None);
                return std::ops::ControlFlow::Break(glib::ExitCode::SUCCESS);
            }
            window::set_start_empty();
        }
        std::ops::ControlFlow::Continue(())
    });
    app.connect_startup(startup);
    app.connect_activate(window::activate);
    app.connect_open(|app, files, _| {
        let paths: Vec<_> = files.iter().filter_map(|f| f.path()).collect();
        window::open_paths(app, &paths);
    });
    app.run()
}

fn startup(app: &adw::Application) {
    sourceview::init();
    let display = gdk::Display::default().expect("display");
    gtk::IconTheme::for_display(&display).add_resource_path("/io/github/Kelvinlby/GittyHub/icons");
    let css = gtk::CssProvider::new();
    css.load_from_resource("/io/github/Kelvinlby/GittyHub/style.css");
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    settings::apply_appearance();
    gtk::Window::set_default_icon_name(APP_ID);

    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(|_, _| window::quit_all());
    let new_window = gio::SimpleAction::new("new-window", None);
    new_window.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, _| window::new_window(&app)
    ));
    app.add_action(&new_window);
    let about = gio::SimpleAction::new("about", None);
    about.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, _| {
            let dialog = adw::AboutDialog::builder()
                .application_name("GittyHub")
                .application_icon(APP_ID)
                .developer_name("GittyHub contributors")
                .version(env!("CARGO_PKG_VERSION"))
                .website("https://github.com/Kelvinlby/gittyhub")
                .issue_url("https://github.com/Kelvinlby/gittyhub/issues")
                .license_type(gtk::License::Gpl30)
                .comments("A Git client with deep GitHub integration, powered by git and gh.")
                .build();
            dialog.present(app.active_window().as_ref());
        }
    ));
    let preferences = gio::SimpleAction::new("preferences", None);
    preferences.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, _| prefs::show(app.active_window().as_ref())
    ));
    for (name, delta) in [
        ("zoom-in", Some(0.1)),
        ("zoom-out", Some(-0.1)),
        ("zoom-reset", None),
    ] {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| {
            settings::step_zoom(delta);
        });
        app.add_action(&a);
    }
    app.add_action(&quit);
    app.add_action(&about);
    app.add_action(&preferences);

    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.new-window", &["<Control>n"]);
    app.set_accels_for_action(
        "app.zoom-in",
        &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
    );
    app.set_accels_for_action("app.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
    app.set_accels_for_action("app.zoom-reset", &["<Control>0", "<Control>KP_0"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.open-local", &["<Control>o"]);
    app.set_accels_for_action("win.open-remote", &["<Control><Shift>o"]);
    app.set_accels_for_action("win.clone", &["<Control><Shift>n"]);
    app.set_accels_for_action("win.new-tab", &["<Control>t"]);
    app.set_accels_for_action("win.close-tab", &["<Control>w"]);
    app.set_accels_for_action("win.refresh", &["F5", "<Control>r"]);
    app.set_accels_for_action("win.fetch", &["<Control><Shift>f"]);
    app.set_accels_for_action("win.pull", &["<Control><Shift>p"]);
    app.set_accels_for_action("win.push", &["<Control>p"]);
}

mod diff;
mod form;
mod open;
mod prefs;
mod repo;
mod settings;
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
    app.connect_startup(startup);
    app.connect_activate(|app| {
        window::Window::create(app, &[]).present();
    });
    app.connect_open(|app, files, _| {
        let paths: Vec<_> = files.iter().filter_map(|f| f.path()).collect();
        window::Window::create(app, &paths).present();
    });
    app.run()
}

fn startup(app: &adw::Application) {
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
    quit.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, _| {
            for w in app.windows() {
                w.close();
            }
        }
    ));
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
    app.add_action(&quit);
    app.add_action(&about);
    app.add_action(&preferences);

    app.set_accels_for_action("app.quit", &["<Control>q"]);
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
    app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
}

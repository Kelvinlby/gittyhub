mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "io.github.Kelvinlby.GittyHub";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| window::build(app).present());
    app.run()
}

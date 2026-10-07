// GTK/WebKit must initialize and shut down on the process main thread.
// Rust's default test harness runs test bodies on worker threads.
#[path = "../src/web.rs"]
mod web;
#[path = "../src/settings.rs"]
mod settings;
#[cfg(feature = "power")]
#[path = "../src/power.rs"]
mod power;

fn main() {
    if std::env::var("PARTICLEWALL_GUI_TEST").as_deref() != Ok("1") {
        println!("web-lifecycle skipped: set PARTICLEWALL_GUI_TEST=1 with a GTK display");
        return;
    }
    web::tests::web_reapply_reuses_host_and_gpu_teardown_releases_it();
    println!("web-lifecycle passed: 30 reapplications, host reuse, widget release");
}

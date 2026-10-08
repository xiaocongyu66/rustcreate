//! Platform shell: desktop binary and Android cdylib share [`app::run`].

pub mod app;
pub mod game;

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    env_logger::init();
    log::info!("mcv starting (android)");
    let _ = pollster::block_on(app::run(Some(app)));
}

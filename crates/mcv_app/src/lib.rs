//! Platform shell: desktop binary and Android cdylib share [`app::run`].

pub mod app;
pub mod game;
pub mod touch;

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: android_activity::AndroidApp) {
    // 日志进 logcat（tag RustMcv）：adb logcat -s RustMcv
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("RustMcv"),
    );
    // panic 也写进 logcat，否则闪退无线索
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));
    log::info!("mcv starting (android)");
    if let Err(e) = pollster::block_on(app::run(Some(app))) {
        log::error!("mcv run failed: {e}");
    }
    // winit 的 EVENT_LOOP_CREATED 是进程级 flag：run 返回后进程若驻留，
    // 再次启动 activity 会复用进程并拿不到 event loop（表现为秒退）。
    // 因此 run 返回即退出，让下次启动是干净的进程。
    std::process::exit(0);
}

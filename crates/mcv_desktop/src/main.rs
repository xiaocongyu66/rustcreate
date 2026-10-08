fn main() {
    env_logger::init();
    log::info!("mcv starting (desktop)");
    if let Err(e) = pollster::block_on(mcv_app::app::run(None)) {
        log::error!("fatal: {e}");
        std::process::exit(1);
    }
}

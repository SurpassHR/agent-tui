use crate::app::App;

impl Drop for App {
    fn drop(&mut self) {
        if self.running {
            tracing::warn!("App dropped while running — 确保主循环已调用 terminate()");
        }
    }
}

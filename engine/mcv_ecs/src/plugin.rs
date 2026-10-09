//! 插件:一组"组件注册 + 系统注册 + 资源初始化"的打包单元(Godot
//! editor plugin / Bevy plugin 同型),装配在启动期一次性完成。

use crate::app::App;

pub trait Plugin {
    fn build(&self, app: &mut App);
}

/// 函数即插件:`add_plugin(plain_plugin(|app| { ... }))`。
/// 重复 build(不应发生)只执行首次——闭包只有一份。
pub fn plain_plugin(f: impl FnOnce(&mut App) + 'static) -> impl Plugin {
    type Build = Box<dyn FnOnce(&mut App)>;
    struct Plain(std::cell::RefCell<Option<Build>>);
    impl Plugin for Plain {
        fn build(&self, app: &mut App) {
            if let Some(f) = self.0.borrow_mut().take() {
                f(app);
            }
        }
    }
    Plain(std::cell::RefCell::new(Some(Box::new(f))))
}

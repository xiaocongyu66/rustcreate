//! 类型化事件总线(Godot signal 的帧语义版):生产者 [`Events::send`]
//! 投递到写缓冲;调度器在**每个阶段边界** [`EventBus::rotate_all`] 把写
//! 缓冲翻到读侧——同阶段发送的事件本阶段读不到,下一阶段可见,再下一阶段
//! 自动清空。双缓冲避免"迭代中发消息"改坏读侧(信号发射前快照的同款语义,
//! 只是用整帧批次代替逐槽快照)。

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::slice;

/// 单类型事件通道:双缓冲。读侧迭代期间写侧随便 send,互不干扰。
pub struct Events<E> {
    reading: Vec<E>,
    writing: Vec<E>,
}

impl<E> Events<E> {
    fn new() -> Self {
        Self {
            reading: Vec::new(),
            writing: Vec::new(),
        }
    }

    /// 投递事件:下个阶段边界起可见,持续一个阶段后自动失效。
    pub fn send(&mut self, e: E) {
        self.writing.push(e);
    }

    /// 当前可见事件(读侧快照,迭代期间可继续 send)。
    pub fn read(&self) -> slice::Iter<'_, E> {
        self.reading.iter()
    }

    /// 整批取走读侧(消费即清,适合阶段结束后一次性结算)。
    pub fn take(&mut self) -> Vec<E> {
        std::mem::take(&mut self.reading)
    }

    pub fn len(&self) -> usize {
        self.reading.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reading.is_empty()
    }

    fn rotate(&mut self) {
        std::mem::swap(&mut self.reading, &mut self.writing);
        self.writing.clear();
    }
}

/// 类型擦除通道面,供 [`EventBus::rotate_all`] 统一翻转。
trait Channel: Any {
    fn rotate(&mut self);
    fn as_any(&mut self) -> &mut dyn Any;
}

impl<E: 'static> Channel for Events<E> {
    fn rotate(&mut self) {
        Events::rotate(self);
    }
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// 事件通道注册表:通道惰性创建(首次 `channel::<E>` 时),阶段边界统一翻转,
/// 未用过的通道零成本。
#[derive(Default)]
pub struct EventBus {
    chans: HashMap<TypeId, Box<dyn Channel>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取(必要时惰性建)`E` 的通道。
    pub fn channel<E: 'static>(&mut self) -> &mut Events<E> {
        self.chans
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(Events::<E>::new()))
            .as_any()
            .downcast_mut::<Events<E>>()
            .expect("TypeId→事件通道类型映射被破坏")
    }

    /// 阶段边界:全通道写→读翻转(上一批读侧作废)。
    pub fn rotate_all(&mut self) {
        for c in self.chans.values_mut() {
            c.rotate();
        }
    }
}

//! 容器槽位状态机：Slot 矩形表 + picked 物品跟随光标 + 左键收/放 +
//! 右键分半 + shift 快移 + 三类拖拽分发。纯 CPU 数据结构 + 事件输出，
//! **不含游戏逻辑**（物品注册表/玩家/配方门都在游戏层）。
//!
//! 机制对照（26.1）：
//! - `AbstractContainerScreen`（screens/inventory/AbstractContainerScreen.java）
//!   的桌面鼠标路径：mouseClicked（:312-386，PICKUP/QUICK_MOVE/THROW
//!   分类 + 拖拽起手）、mouseDragged（:408-441，quickCraftSlots 收集）、
//!   mouseReleased（:444-539，双击 PICKUP_ALL / 拖拽三段分发 / 松手
//!   PICKUP）；isHovering（:546-556，16x16 槽外扩 1）；picked 物品
//!   跟随光标（extractCarriedItem :126-144，偏移 -8,-8）；
//! - `AbstractContainerMenu.doClick`（world/inventory/AbstractContainerMenu.java
//!   :334-558）与 `Slot.safeInsert/tryRemove`（Slot.java:97-149）：
//!   收/放/分半/合并/交换的数值语义；
//! - 拖拽三分发 = `quickCraftType` 0/1/2（:41-43 CHARITABLE/GREEDY/
//!   CLONE）：左键均分、右键每个 1、克隆（仅创造，isValidQuickcraftType
//!   :714-720）；分发协议 = getQuickcraftMask header(0/1/2)（:44-46、
//!   :702-712），quickCraftToSlots 三段发送（AbstractContainerScreen
//!   .java:671-679）；
//! - `moveItemStackTo`（:638-700）：先同类合并后空槽放置，backwards
//!   从尾向头；
//! - 面板外点击 = SLOT_CLICKED_OUTSIDE -999（:40）。
//!
//! 触屏专属分支（touchscreen quickdrop/snapback，:411-434、:489-519）
//! 未移植，列入迁移清单。

use crate::context::Rect;

/// 面板外点击（AbstractContainerMenu.java:40 SLOT_CLICKED_OUTSIDE）。
pub const SLOT_OUTSIDE: i32 = -999;

/// 槽位视觉尺寸（Slot 16x16，命中外扩 1）。
pub const SLOT_PX: f32 = 16.0;
/// picked 物品跟随光标偏移（extractCarriedItem：mouseX-8, mouseY-8）。
pub const CARRIED_OFFSET: f32 = 8.0;

/// 拖拽分发类型（AbstractContainerMenu.java:41-43）。
pub const QC_CHARITABLE: u8 = 0; // 左键均分
pub const QC_GREEDY: u8 = 1; // 右键每个 1
pub const QC_CLONE: u8 = 2; // 克隆（仅创造）

/// 物品栈镜像：mcv_ui 不引物品注册表，用最小载体（kind=物品 id；
/// count=0 即空栈，同 ItemStack.count 0 = empty 约定）。组件级
/// 相等性（isSameItemSameComponents 的 components 部分）未建模，
/// 同 kind 即同类。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StackRef {
    pub kind: u32,
    pub count: u32,
}

impl StackRef {
    pub fn new(kind: u32, count: u32) -> Self {
        Self { kind, count }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0 || self.kind == 0
    }
}

/// 槽位类别（Slot 子集）：Normal 可收可放；Result 只出不进
/// （ResultSlot 的 mayPlace=false 语义，配方成立性由游戏层把关）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Normal,
    Result,
}

/// 槽位定义：面板内逻辑像素坐标 + quickMove 路由分组。
#[derive(Clone, Copy, Debug)]
pub struct SlotDef {
    pub x: f32,
    pub y: f32,
    /// 分组（= 原版 Slot.container 的身份：同组 = 同容器，双击快移/路由用）。
    pub group: u16,
    pub kind: SlotKind,
}

impl SlotDef {
    pub fn new(x: f32, y: f32, group: u16) -> Self {
        Self {
            x,
            y,
            group,
            kind: SlotKind::Normal,
        }
    }

    pub fn result(x: f32, y: f32, group: u16) -> Self {
        Self {
            x,
            y,
            group,
            kind: SlotKind::Result,
        }
    }
}

/// 菜单命令（= 原版 screen → server 的 `handleContainerInput` 载荷；
/// 单机下游戏层直接把它喂给本 crate 的 `menu::MenuModel` 或自己的
/// 背包适配层）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    Click {
        /// 槽位下标；[`SLOT_OUTSIDE`] = 面板外。
        slot: i32,
        /// 0 = 左键（PRIMARY），1 = 右键（SECONDARY）。
        button: u8,
        input: Input,
    },
}

/// `ContainerInput`（world/inventory/ContainerInput.java）子集：
/// SWAP（快捷栏槽位键对调）未建模，见报告差异表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// PICKUP(0)：收/放/合并/交换/分半（右键）。
    Pickup,
    /// QUICK_MOVE(1)：shift 快移（路由到 router 指定区间）。
    QuickMove,
    /// THROW(4)：丢出（面板外点击或丢弃键）。
    Throw,
    /// CLONE(3)：创造中键复制整组。
    Clone,
    /// QUICK_CRAFT(5)：拖拽分发三段（mask = header | type<<2）。
    QuickCraft(u8),
    /// PICKUP_ALL(6)：双击收拢同类。
    PickupAll,
}

/// getQuickcraftHeader（AbstractContainerMenu.java:706-708）。
pub fn get_quickcraft_header(mask: u8) -> u8 {
    mask & 3
}

/// getQuickcraftType（:702-704）。
pub fn get_quickcraft_type(mask: u8) -> u8 {
    mask >> 2 & 3
}

/// getQuickcraftMask（:710-712）。
pub fn get_quickcraft_mask(header: u8, qc_type: u8) -> u8 {
    header & 3 | (qc_type & 3) << 2
}

/// getQuickCraftPlaceCount（:734-741）：n 格分摊量。
pub fn quickcraft_place_count(n: u32, qc_type: u8, carried_count: u32, item_max: u32) -> u32 {
    match qc_type {
        QC_CHARITABLE => carried_count / n.max(1),
        QC_GREEDY => 1,
        QC_CLONE => item_max,
        _ => carried_count,
    }
}

/// 容器界面状态机（屏幕侧状态 + 菜单镜像一体；单机语义下镜像即权威，
/// 游戏层打开时 `set_items` 灌入、关闭时读回；中途事件用
/// `take_cmds`/`take_drops` 消费）。
pub struct ContainerScreen {
    /// 面板矩形（hasClickedOutside :403-405 的判定域）。
    pub panel: Rect,
    /// 槽位表（下标 = 原版 Slot.index）。
    pub slots: Vec<SlotDef>,
    /// 槽位物品镜像（与 slots 等长）。
    pub items: Vec<StackRef>,
    /// picked 物品（menu.getCarried()）。
    pub carried: StackRef,
    /// 容器堆叠上限（container.getMaxStackSize()，默认 64）。
    pub container_max: u32,
    /// 创造/无限物品（hasInfiniteMaterials：CLONE/类型 2 拖拽的门槛）。
    pub infinite_materials: bool,
    max_stack: fn(u32) -> u32,
    /// quickMove 路由（= 各 Menu 的 quickMoveStack 重载）：分组 →
    /// (目标区间, 是否倒序)。
    router: Box<dyn Fn(u16) -> (usize, usize, bool)>,
    // ---- 屏幕侧交互状态（AbstractContainerScreen.java:49-67 对应）----
    /// isQuickCrafting（拖拽进行中）。
    is_quickcrafting: bool,
    /// menu 端 QUICK_CRAFT 协议状态（0/1/2，doQuickCraft 流水）。
    quickcraft_status: u8,
    quickcraft_type: u8,
    quickcraft_button: u8,
    quickcraft_slots: Vec<usize>,
    skip_next_release: bool,
    last_click_slot: Option<usize>,
    doubleclick: bool,
    last_quick_moved: StackRef,
    drops: Vec<StackRef>,
    /// 待消费命令（take_cmds）。
    cmds: Vec<MenuCmd>,
}

impl ContainerScreen {
    /// 构造：`max_stack(kind) -> u32` 物品堆叠上限（游戏层给注册表），
    /// `router` quickMove 分组路由。
    pub fn new(
        panel: Rect,
        slots: Vec<SlotDef>,
        max_stack: fn(u32) -> u32,
        router: Box<dyn Fn(u16) -> (usize, usize, bool)>,
    ) -> Self {
        let items = vec![StackRef::default(); slots.len()];
        Self {
            panel,
            slots,
            items,
            carried: StackRef::default(),
            container_max: 64,
            infinite_materials: false,
            max_stack,
            router,
            is_quickcrafting: false,
            quickcraft_status: 0,
            quickcraft_type: 0,
            quickcraft_button: 0,
            quickcraft_slots: Vec::new(),
            skip_next_release: true, // 构造即 true（AbstractContainerScreen:79）
            last_click_slot: None,
            doubleclick: false,
            last_quick_moved: StackRef::default(),
            drops: Vec::new(),
            cmds: Vec::new(),
        }
    }

    /// 打开/外部变更时灌镜像（长度对齐槽位表）。
    pub fn set_items(&mut self, items: Vec<StackRef>) {
        self.items = items;
        self.items.resize(self.slots.len(), StackRef::default());
    }

    /// 取走累积命令。
    pub fn take_cmds(&mut self) -> Vec<MenuCmd> {
        std::mem::take(&mut self.cmds)
    }

    /// 取走丢出物（THROW / 面板外点击；游戏层负责生成掉落物实体）。
    pub fn take_drops(&mut self) -> Vec<StackRef> {
        std::mem::take(&mut self.drops)
    }

    /// 拖拽目标槽集合（预览高亮用）。
    pub fn quickcraft_slots(&self) -> &[usize] {
        &self.quickcraft_slots
    }

    /// 悬停槽位（getHoveredSlot；16x16 外扩 1，AbstractContainerScreen:546-556）。
    pub fn hovered_at(&self, x: f32, y: f32) -> Option<usize> {
        for (i, s) in self.slots.iter().enumerate() {
            if x >= s.x - 1.0
                && x < s.x + SLOT_PX + 1.0
                && y >= s.y - 1.0
                && y < s.y + SLOT_PX + 1.0
            {
                return Some(i);
            }
        }
        None
    }

    /// 面板外（hasClickedOutside :403-405）。
    pub fn clicked_outside(&self, x: f32, y: f32) -> bool {
        !self.panel.contains(x, y)
    }

    /// picked 物品绘制位（跟随光标，extractCarriedItem :126-144）。
    pub fn carried_draw(&self, mx: f32, my: f32) -> (f32, f32, &StackRef) {
        (mx - CARRIED_OFFSET, my - CARRIED_OFFSET, &self.carried)
    }

    /// 拖拽中的目标槽预览（extractSlot :234-254 的数量合成；
    /// 单格拖拽不显示预览，同原版 quickCraftSlots.size() == 1 分支）。
    pub fn quickcraft_preview(&self, i: usize) -> Option<StackRef> {
        if !self.is_quickcrafting
            || !self.quickcraft_slots.contains(&i)
            || self.carried.is_empty()
            || self.quickcraft_slots.len() == 1
        {
            return None;
        }
        if !self.can_item_quick_replace(i, &self.carried, true) || !self.can_drag_to(i) {
            return None;
        }
        let carry = self.items[i].count;
        let cap = self.slot_cap(self.carried.kind);
        let n = self.quickcraft_slots.len() as u32;
        let new_count = (quickcraft_place_count(
            n,
            self.quickcraft_type,
            self.carried.count,
            (self.max_stack)(self.carried.kind),
        ) + carry)
            .min(cap);
        Some(StackRef {
            kind: self.carried.kind,
            count: new_count,
        })
    }

    // ---- 输入（桌面鼠标路径）----

    /// 按下（mouseClicked :312-386）。
    pub fn mouse_down(&mut self, x: f32, y: f32, button: u8, shift: bool, double_click: bool) {
        if button > 1 {
            // 中键 CLONE / 侧键热栏对调未建模（差异表）
            return;
        }
        let slot = self.hovered_at(x, y);
        self.doubleclick = self.last_click_slot == slot && double_click;
        self.skip_next_release = false;
        let outside = self.clicked_outside(x, y);
        let slot_id = match slot {
            Some(i) => i as i32,
            None if outside => SLOT_OUTSIDE,
            None => -1,
        };
        if slot_id != -1 && !self.is_quickcrafting {
            if self.carried.is_empty() {
                let input = if shift && slot_id != SLOT_OUTSIDE {
                    // 快移基准（lastQuickMoved，:358）
                    self.last_quick_moved = slot
                        .and_then(|i| (!self.items[i].is_empty()).then(|| self.items[i].clone()))
                        .unwrap_or_default();
                    Input::QuickMove
                } else if slot_id == SLOT_OUTSIDE {
                    Input::Throw
                } else {
                    Input::Pickup
                };
                self.do_click(slot_id, button, input);
                self.cmds.push(MenuCmd::Click {
                    slot: slot_id,
                    button,
                    input,
                });
                self.skip_next_release = true;
            } else {
                // 有物在手 → 起拖（:369-379）
                self.quickcrafting_start(button);
            }
        }
        self.last_click_slot = slot;
    }

    /// 拖动（mouseDragged :408-441 桌面分支：收集 quickCraftSlots）。
    pub fn mouse_drag(&mut self, x: f32, y: f32, _button: u8) {
        if let Some(i) = self.hovered_at(x, y)
            && self.should_add_to_quickcraft(i)
            && !self.quickcraft_slots.contains(&i)
        {
            self.quickcraft_slots.push(i);
        }
    }

    /// 松开（mouseReleased :444-539）。
    pub fn mouse_up(&mut self, x: f32, y: f32, button: u8, shift: bool) {
        let slot = self.hovered_at(x, y);
        let outside = self.clicked_outside(x, y);
        let slot_id = match slot {
            Some(i) => i as i32,
            None if outside => SLOT_OUTSIDE,
            None => -1,
        };
        if self.doubleclick
            && button == 0
            && let Some(si) = slot
        {
            // 双击：收拢同类（PICKUP_ALL）；shift = 同容器同物快移（:458-475）
            if shift {
                let group = self.slots[si].group;
                let moved = self.last_quick_moved.clone();
                if !moved.is_empty() {
                    for i in 0..self.slots.len() {
                        if self.slots[i].group == group
                            && i != si
                            && !self.items[i].is_empty()
                            && self.items[i].kind == moved.kind
                            && self.can_item_quick_replace(i, &moved, true)
                        {
                            self.do_click(i as i32, button, Input::QuickMove);
                            self.cmds.push(MenuCmd::Click {
                                slot: i as i32,
                                button,
                                input: Input::QuickMove,
                            });
                        }
                    }
                }
                self.doubleclick = false;
            } else {
                self.do_click(slot_id, 0, Input::PickupAll);
                self.cmds.push(MenuCmd::Click {
                    slot: slot_id,
                    button: 0,
                    input: Input::PickupAll,
                });
                self.doubleclick = false;
            }
        } else {
            if self.is_quickcrafting && self.quickcraft_button != button {
                // 换键松开 = 取消拖拽（:477-482）
                self.is_quickcrafting = false;
                self.quickcraft_slots.clear();
                self.skip_next_release = true;
                return;
            }
            if self.skip_next_release {
                self.skip_next_release = false;
                return;
            }
            if self.is_quickcrafting && !self.quickcraft_slots.is_empty() {
                // quickCraftToSlots（:671-679）：header(0) → 每槽(1) → end(2)。
                // 原版屏幕侧与菜单侧是两个集合：header 清菜单侧集合后由
                // continue 逐格重灌；本实现集合一体，故先快照再发送。
                let t = self.quickcraft_type;
                let targets: Vec<usize> = self.quickcraft_slots.clone();
                let m0 = get_quickcraft_mask(0, t);
                self.do_click(SLOT_OUTSIDE, m0, Input::QuickCraft(m0));
                self.cmds.push(MenuCmd::Click {
                    slot: SLOT_OUTSIDE,
                    button: m0,
                    input: Input::QuickCraft(m0),
                });
                let m1 = get_quickcraft_mask(1, t);
                for &i in &targets {
                    self.do_click(i as i32, m1, Input::QuickCraft(m1));
                    self.cmds.push(MenuCmd::Click {
                        slot: i as i32,
                        button: m1,
                        input: Input::QuickCraft(m1),
                    });
                }
                let m2 = get_quickcraft_mask(2, t);
                self.do_click(SLOT_OUTSIDE, m2, Input::QuickCraft(m2));
                self.cmds.push(MenuCmd::Click {
                    slot: SLOT_OUTSIDE,
                    button: m2,
                    input: Input::QuickCraft(m2),
                });
            } else if !self.carried.is_empty() {
                // 松手收/放（:523-533）
                let quick_key = shift && slot_id != SLOT_OUTSIDE;
                if quick_key {
                    self.last_quick_moved = slot
                        .and_then(|i| (!self.items[i].is_empty()).then(|| self.items[i].clone()))
                        .unwrap_or_default();
                }
                let input = if quick_key {
                    Input::QuickMove
                } else {
                    Input::Pickup
                };
                self.do_click(slot_id, button, input);
                self.cmds.push(MenuCmd::Click {
                    slot: slot_id,
                    button,
                    input,
                });
            }
        }
        self.is_quickcrafting = false; // isQuickCrafting = false（:537）
    }

    /// 键盘丢弃（keyPressed :603-608 的 THROW；游戏层按 drop 键调用）。
    pub fn key_throw(&mut self, hovered: usize, whole_stack: bool) {
        if self.carried.is_empty() {
            let button = if whole_stack { 1 } else { 0 };
            self.do_click(hovered as i32, button, Input::Throw);
            self.cmds.push(MenuCmd::Click {
                slot: hovered as i32,
                button,
                input: Input::Throw,
            });
        }
    }

    // ---- doClick（AbstractContainerMenu.java:334-558 移植）----

    pub fn do_click(&mut self, slot_index: i32, button: u8, input: Input) {
        match input {
            Input::QuickCraft(mask) => self.do_quickcraft(slot_index, mask),
            _ if self.quickcraft_status != 0 => self.reset_quickcraft(), // :400-401
            Input::Pickup | Input::QuickMove if button <= 1 => {
                self.do_pickup_or_quickmove(slot_index, button, input)
            }
            Input::Clone
                if self.infinite_materials && self.carried.is_empty() && slot_index >= 0 =>
            {
                // :508-513
                let i = slot_index as usize;
                let item = self.items[i].clone();
                if !item.is_empty() {
                    self.carried = StackRef {
                        kind: item.kind,
                        count: self.slot_cap(item.kind),
                    };
                }
            }
            Input::Throw if self.carried.is_empty() && slot_index >= 0 => {
                // :514-534（button 0 = 丢 1，button 1 = 整组）
                let i = slot_index as usize;
                let amount = if button == 0 { 1 } else { self.items[i].count };
                let taken = self.try_remove(i, amount);
                if !taken.is_empty() {
                    self.drops.push(taken);
                }
            }
            Input::PickupAll if slot_index >= 0 => self.do_pickup_all(slot_index, button),
            _ => {}
        }
    }

    fn do_pickup_or_quickmove(&mut self, slot_index: i32, button: u8, input: Input) {
        if slot_index == SLOT_OUTSIDE {
            // :404-412
            if !self.carried.is_empty() {
                if button == 0 {
                    self.drops.push(self.carried.clone());
                    self.carried = StackRef::default();
                } else {
                    let one = StackRef {
                        kind: self.carried.kind,
                        count: 1,
                    };
                    self.carried.count -= 1;
                    self.drops.push(one);
                }
            }
            return;
        }
        if slot_index < 0 {
            return;
        }
        let i = slot_index as usize;
        if matches!(input, Input::QuickMove) {
            // :413-427
            if !self.may_pickup(i) {
                return;
            }
            let mut moved = self.quick_move_stack(i);
            let mut guard = 0u8;
            while !moved.is_empty() && !self.items[i].is_empty() && self.items[i].kind == moved.kind
            {
                let prev = self.items[i].count;
                moved = self.quick_move_stack(i);
                guard += 1;
                // 防御：原版依赖 quickMoveStack 返回空收敛，此处再兜底
                if guard >= 64 || self.items[i].count >= prev {
                    break;
                }
            }
            return;
        }
        // PICKUP（:428-470）
        let clicked = self.items[i].clone();
        if clicked.is_empty() {
            if !self.carried.is_empty() {
                let amount = if button == 0 { self.carried.count } else { 1 };
                self.safe_insert_carried(i, amount);
            }
        } else if self.may_pickup(i) {
            if self.carried.is_empty() {
                // 收取：左键整组 / 右键分半向上取整（:445 ceil(count/2)）
                let amount = if button == 0 {
                    clicked.count
                } else {
                    clicked.count.div_ceil(2)
                };
                self.carried = self.try_remove(i, amount);
            } else if self.may_place(i, &self.carried) {
                if self.items[i].kind == self.carried.kind {
                    // 同类：左键全放 / 右键放 1（:452-454）
                    let amount = if button == 0 { self.carried.count } else { 1 };
                    self.safe_insert_carried(i, amount);
                } else if self.carried.count <= self.slot_cap(self.carried.kind) {
                    // 异类且槽装得下 → 交换（:455-458）
                    let carried = std::mem::take(&mut self.carried);
                    self.items[i] = carried;
                    self.carried = clicked;
                }
            } else if self.items[i].kind == self.carried.kind {
                // 槽不许放但同类（如结果槽满载）：向光标并（:459-465）
                let space = self
                    .item_max(self.carried.kind)
                    .saturating_sub(self.carried.count);
                let want = clicked.count.min(space);
                let taken = self.try_remove(i, want);
                self.carried.count += taken.count;
            }
        }
    }

    fn do_pickup_all(&mut self, slot_index: i32, button: u8) {
        // :535-557；门槛：目标槽为空或不可取（双击先收空了该槽）
        let i = slot_index as usize;
        if self.carried.is_empty() || (!self.items[i].is_empty() && self.may_pickup(i)) {
            return;
        }
        let item_max = self.item_max(self.carried.kind);
        let (start, step) = if button == 0 {
            (0i64, 1i64)
        } else {
            (self.items.len() as i64 - 1, -1i64)
        };
        for pass in 0..2 {
            let mut k = start;
            while k >= 0 && (k as usize) < self.items.len() && self.carried.count < item_max {
                let t = k as usize;
                let target = self.items[t].clone();
                // canItemQuickReplace(target, carried, true)：非空同类即可；
                // pass 0 跳过满载槽（:549）
                if !target.is_empty()
                    && target.kind == self.carried.kind
                    && self.may_pickup(t)
                    && (pass != 0 || target.count != self.slot_cap(target.kind))
                {
                    let room = item_max - self.carried.count;
                    let removed = self.try_remove(t, room);
                    self.carried.count += removed.count;
                }
                k += step;
            }
        }
    }

    fn do_quickcraft(&mut self, slot_index: i32, mask: u8) {
        // :336-399。前两个分支都只做 reset（原版为 else-if 链，此处合并
        // 条件以过 clippy::if_same_then_else，行为不变）
        let expected = self.quickcraft_status;
        self.quickcraft_status = get_quickcraft_header(mask);
        if ((expected != 1 || self.quickcraft_status != 2) && expected != self.quickcraft_status)
            || self.carried.is_empty()
        {
            self.reset_quickcraft();
        } else if self.quickcraft_status == 0 {
            self.quickcraft_type = get_quickcraft_type(mask);
            if self.is_valid_quickcraft_type(self.quickcraft_type) {
                self.quickcraft_status = 1;
                self.quickcraft_slots.clear();
            } else {
                self.reset_quickcraft();
            }
        } else if self.quickcraft_status == 1 {
            if slot_index >= 0 && (slot_index as usize) < self.slots.len() {
                let i = slot_index as usize;
                if self.can_item_quick_replace(i, &self.carried, true)
                    && self.may_place(i, &self.carried)
                    && (self.quickcraft_type == QC_CLONE
                        || self.carried.count > self.quickcraft_slots.len() as u32)
                    && self.can_drag_to(i)
                    && !self.quickcraft_slots.contains(&i)
                {
                    self.quickcraft_slots.push(i);
                }
            }
        } else if self.quickcraft_status == 2 {
            if !self.quickcraft_slots.is_empty() {
                if self.quickcraft_slots.len() == 1 {
                    // 单格拖拽 = 一次普通收/放（button = 拖拽类型，:362-367）
                    let slot = self.quickcraft_slots[0];
                    self.reset_quickcraft();
                    self.do_click(slot as i32, self.quickcraft_type, Input::Pickup);
                    return;
                }
                let source = self.carried.clone();
                if source.is_empty() {
                    self.reset_quickcraft();
                    return;
                }
                let mut remaining = self.carried.count;
                let n = self.quickcraft_slots.len() as u32;
                let item_max = (self.max_stack)(source.kind);
                for &i in &self.quickcraft_slots {
                    if i < self.items.len()
                        && self.can_item_quick_replace(i, &self.carried, true)
                        && self.may_place(i, &self.carried)
                        && (self.quickcraft_type == QC_CLONE || self.carried.count >= n)
                        && self.can_drag_to(i)
                    {
                        let carry = self.items[i].count;
                        let cap = self.slot_cap(source.kind);
                        let new_count = (quickcraft_place_count(
                            n,
                            self.quickcraft_type,
                            self.carried.count,
                            item_max,
                        ) + carry)
                            .min(cap);
                        remaining = remaining.saturating_sub(new_count - carry);
                        self.items[i] = StackRef {
                            kind: source.kind,
                            count: new_count,
                        };
                    }
                }
                self.carried.count = remaining;
            }
            self.reset_quickcraft();
        } else {
            self.reset_quickcraft();
        }
    }

    // ---- 槽位原语（Slot.java）----

    fn item_max(&self, kind: u32) -> u32 {
        (self.max_stack)(kind)
    }

    /// Slot.getMaxStackSize(item)：容器上限与物品上限取小。
    fn slot_cap(&self, kind: u32) -> u32 {
        self.container_max.min(self.item_max(kind))
    }

    fn may_pickup(&self, _i: usize) -> bool {
        // Result 槽的配方成立性门在游戏层（take_result），UI 层恒可取
        true
    }

    fn may_place(&self, i: usize, _stack: &StackRef) -> bool {
        self.slots[i].kind != SlotKind::Result
    }

    fn can_drag_to(&self, _i: usize) -> bool {
        true // AbstractContainerMenu.canDragTo 默认（:743-745）
    }

    /// canItemQuickReplace（:727-732）。
    fn can_item_quick_replace(&self, i: usize, item: &StackRef, ignore_size: bool) -> bool {
        if self.items[i].is_empty() {
            return true;
        }
        if self.items[i].kind == item.kind {
            let extra = if ignore_size { 0 } else { item.count };
            self.items[i].count + extra <= self.item_max(item.kind)
        } else {
            false
        }
    }

    /// Slot.tryRemove（Slot.java:97-117）：取走 ≤amount。
    fn try_remove(&mut self, i: usize, amount: u32) -> StackRef {
        let item = &mut self.items[i];
        if item.is_empty() {
            return StackRef::default();
        }
        let n = amount.min(item.count);
        let kind = item.kind;
        item.count -= n;
        StackRef { kind, count: n }
    }

    /// Slot.safeInsert（Slot.java:125-149）对 carried 的变体。
    fn safe_insert_carried(&mut self, i: usize, amount: u32) {
        if self.carried.is_empty() || !self.may_place(i, &self.carried) {
            return;
        }
        if !self.items[i].is_empty() && self.items[i].kind != self.carried.kind {
            return;
        }
        let cap = self
            .slot_cap(self.carried.kind)
            .saturating_sub(self.items[i].count);
        let transfer = amount.min(self.carried.count).min(cap);
        if transfer == 0 {
            return;
        }
        if self.items[i].is_empty() {
            self.items[i] = StackRef {
                kind: self.carried.kind,
                count: transfer,
            };
        } else {
            self.items[i].count += transfer;
        }
        self.carried.count -= transfer;
    }

    /// moveItemStackTo（:638-700）：先同类合并后空槽放置；`skip` 槽位
    /// 跳过（防快移自并；无跳过传 usize::MAX）。
    fn move_item_stack_to(
        &mut self,
        stack: &mut StackRef,
        start: usize,
        end: usize,
        backwards: bool,
        skip: usize,
    ) -> bool {
        let mut changed = false;
        // pass 1：合并同类（:645-671）
        if !stack.is_empty() {
            let mut dest = if backwards {
                end.wrapping_sub(1)
            } else {
                start
            };
            while !stack.is_empty() && (if backwards { dest >= start } else { dest < end }) {
                if dest < self.items.len() && dest != skip {
                    let target = self.items[dest].clone();
                    if !target.is_empty() && target.kind == stack.kind {
                        let cap = self.slot_cap(stack.kind);
                        let total = target.count + stack.count;
                        if total <= cap {
                            stack.count = 0;
                            self.items[dest].count = total;
                            changed = true;
                        } else if target.count < cap {
                            let move_n = cap - target.count;
                            stack.count -= move_n;
                            self.items[dest].count = cap;
                            changed = true;
                        }
                    }
                }
                dest = if backwards {
                    dest.wrapping_sub(1)
                } else {
                    dest + 1
                };
            }
        }
        // pass 2：空槽放置（:673-697）
        if !stack.is_empty() {
            let mut dest = if backwards {
                end.wrapping_sub(1)
            } else {
                start
            };
            while if backwards { dest >= start } else { dest < end } {
                if dest < self.items.len()
                    && dest != skip
                    && self.items[dest].is_empty()
                    && self.may_place(dest, stack)
                {
                    let cap = self.slot_cap(stack.kind);
                    let n = stack.count.min(cap);
                    self.items[dest] = StackRef {
                        kind: stack.kind,
                        count: n,
                    };
                    stack.count -= n;
                    changed = true;
                    break;
                }
                dest = if backwards {
                    dest.wrapping_sub(1)
                } else {
                    dest + 1
                };
            }
        }
        changed
    }

    /// quickMoveStack：路由 + 搬运 + 回写；返回"源槽剩余同类"以驱动
    /// doClick 的收敛循环（源清空/换物返回空）。
    fn quick_move_stack(&mut self, from: usize) -> StackRef {
        let before_kind = self.items[from].kind;
        if self.items[from].is_empty() {
            return StackRef::default();
        }
        let (start, end, backwards) = (self.router)(self.slots[from].group);
        let mut stack = self.items[from].clone();
        let moved = self.move_item_stack_to(&mut stack, start, end, backwards, from);
        if moved {
            self.items[from] = stack;
        }
        if self.items[from].is_empty() || self.items[from].kind != before_kind {
            StackRef::default()
        } else {
            self.items[from].clone()
        }
    }

    // ---- 拖拽辅助 ----

    fn quickcrafting_start(&mut self, button: u8) {
        // isQuickCrafting = true + button/type（:369-379）；类型 2 需中键
        // CLONE（未建模），此处只有 0/1
        self.is_quickcrafting = true;
        self.quickcraft_button = button;
        self.quickcraft_type = match button {
            0 => QC_CHARITABLE,
            _ => QC_GREEDY,
        };
        self.quickcraft_slots.clear();
    }

    fn should_add_to_quickcraft(&self, i: usize) -> bool {
        // AbstractContainerScreen.shouldAddSlotToQuickCraft :662-669
        self.is_quickcrafting
            && !self.carried.is_empty()
            && (self.carried.count > self.quickcraft_slots.len() as u32
                || self.quickcraft_type == QC_CLONE)
            && self.can_item_quick_replace(i, &self.carried, true)
            && self.may_place(i, &self.carried)
            && self.can_drag_to(i)
    }

    fn reset_quickcraft(&mut self) {
        // AbstractContainerMenu.resetQuickCraft（:722-725，不触碰 type）
        self.quickcraft_status = 0;
        self.quickcraft_slots.clear();
    }

    fn is_valid_quickcraft_type(&self, t: u8) -> bool {
        // isValidQuickcraftType（:714-720）
        t == QC_CHARITABLE || t == QC_GREEDY || (t == QC_CLONE && self.infinite_materials)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试菜单：槽 0..2 = 容器（组 0），槽 3 = 结果槽（组 0），
    /// 槽 4..12 = 背包（组 1）。路由：容器 → 背包区间，背包 → 容器
    /// 区间（不含结果槽）。
    fn screen() -> ContainerScreen {
        let mut slots = Vec::new();
        for i in 0..3 {
            slots.push(SlotDef::new(i as f32 * 18.0, 0.0, 0));
        }
        slots.push(SlotDef::result(3.0 * 18.0, 0.0, 0));
        for i in 0..8 {
            slots.push(SlotDef::new(i as f32 * 18.0, 18.0, 1));
        }
        ContainerScreen::new(
            Rect::new(-4.0, -4.0, 200.0, 60.0),
            slots,
            |_: u32| 64,
            Box::new(|g| match g {
                0 => (4, 12, false), // 容器 → 背包
                _ => (0, 3, false),  // 背包 → 容器（不含结果槽）
            }),
        )
    }

    fn item(k: u32, n: u32) -> StackRef {
        StackRef::new(k, n)
    }

    #[test]
    fn pickup_whole_stack_and_place_back() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 32), StackRef::default(), StackRef::default()]);
        // 左键收
        cs.do_click(0, 0, Input::Pickup);
        assert_eq!(cs.items[0], StackRef::default());
        assert_eq!(cs.carried, item(1, 32));
        // 左键放到空槽
        cs.do_click(1, 0, Input::Pickup);
        assert_eq!(cs.items[1], item(1, 32));
        assert!(cs.carried.is_empty());
        // 左键异类交换
        cs.items[2] = item(2, 5);
        cs.do_click(1, 0, Input::Pickup); // 收 32
        cs.do_click(2, 0, Input::Pickup); // 与 5 交换
        assert_eq!(cs.items[2], item(1, 32));
        assert_eq!(cs.carried, item(2, 5));
    }

    #[test]
    fn right_click_splits_half_and_places_one() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 33), StackRef::default(), StackRef::default()]);
        // 右键分半：33 → ceil/2 = 17 进光标，16 留槽（AbstractContainerMenu:445）
        cs.do_click(0, 1, Input::Pickup);
        assert_eq!(cs.carried, item(1, 17));
        assert_eq!(cs.items[0], item(1, 16));
        // 右键放到空槽：只放 1（:440）
        cs.do_click(1, 1, Input::Pickup);
        assert_eq!(cs.items[1], item(1, 1));
        assert_eq!(cs.carried, item(1, 16));
        // 右键放到同类：再放 1（:452-454）
        cs.do_click(1, 1, Input::Pickup);
        assert_eq!(cs.items[1], item(1, 2));
        assert_eq!(cs.carried, item(1, 15));
    }

    #[test]
    fn same_kind_merges_on_place() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 50), item(1, 30), StackRef::default()]);
        cs.do_click(0, 0, Input::Pickup); // carried 50
        cs.do_click(1, 0, Input::Pickup); // 并进 30 → 槽 64，余 16
        assert_eq!(cs.items[1], item(1, 64));
        assert_eq!(cs.carried, item(1, 16));
    }

    #[test]
    fn shift_quick_moves_between_groups() {
        let mut cs = screen();
        cs.set_items(vec![
            item(1, 40),
            StackRef::default(),
            StackRef::default(),
            StackRef::default(),
            item(1, 50),
            item(2, 3),
            StackRef::default(),
            StackRef::default(),
            StackRef::default(),
            StackRef::default(),
            StackRef::default(),
            StackRef::default(),
        ]);
        // 背包槽 4 的 50 快移到容器（先并 40 → 64，余 26 落空槽）
        cs.do_click(4, 0, Input::QuickMove);
        assert_eq!(cs.items[0], item(1, 64));
        assert_eq!(cs.items[1], item(1, 26));
        assert!(cs.items[4].is_empty());
        // 结果槽也可取走（quickMove 出结果槽 → 组 0 的目标区间 = 背包）
        cs.items[3] = item(3, 1);
        cs.do_click(3, 0, Input::QuickMove);
        assert_eq!(cs.items[4], item(3, 1));
        assert!(cs.items[3].is_empty());
    }

    #[test]
    fn click_outside_drops_carried() {
        let mut cs = screen();
        cs.carried = item(1, 7);
        // 面板外左键松手 = 整组丢（:404-408）
        cs.do_click(SLOT_OUTSIDE, 0, Input::Pickup);
        assert!(cs.carried.is_empty());
        assert_eq!(cs.take_drops(), vec![item(1, 7)]);
        // 面板外右键 = 丢 1（:409-411）
        cs.carried = item(2, 7);
        cs.do_click(SLOT_OUTSIDE, 1, Input::Pickup);
        assert_eq!(cs.carried, item(2, 6));
        assert_eq!(cs.take_drops(), vec![item(2, 1)]);
    }

    #[test]
    fn key_throw_drops_one_or_stack() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 9), StackRef::default(), StackRef::default()]);
        cs.key_throw(0, false);
        assert_eq!(cs.items[0], item(1, 8));
        assert_eq!(cs.take_drops(), vec![item(1, 1)]);
        cs.key_throw(0, true);
        assert!(cs.items[0].is_empty());
        assert_eq!(cs.take_drops(), vec![item(1, 8)]);
    }

    #[test]
    fn result_slot_rejects_place() {
        let mut cs = screen();
        cs.carried = item(1, 10);
        // 结果槽 mayPlace=false：空槽 + 有物在手 → 放不进
        cs.do_click(3, 0, Input::Pickup);
        assert_eq!(cs.carried, item(1, 10));
        assert!(cs.items[3].is_empty());
    }

    #[test]
    fn clone_requires_infinite_materials() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 3), StackRef::default(), StackRef::default()]);
        cs.do_click(0, 0, Input::Clone);
        assert!(cs.carried.is_empty());
        cs.infinite_materials = true;
        cs.do_click(0, 0, Input::Clone);
        assert_eq!(cs.carried, item(1, 64));
    }

    #[test]
    fn double_click_pickup_all_gathers() {
        // 原版双击流程：二击按下先把整组收空目标槽，松手 PICKUP_ALL
        // 才过 `!slot.hasItem()` 门槛（AbstractContainerMenu:538）。
        let mut cs = screen();
        cs.set_items(vec![
            item(1, 10),
            item(1, 20),
            item(2, 5),
            StackRef::default(),
            item(1, 30),
        ]);
        // 1) 左键收槽 0 → carried 10
        cs.mouse_down(0.0, 0.0, 0, false, false);
        cs.mouse_up(0.0, 0.0, 0, false);
        assert_eq!(cs.carried, item(1, 10));
        // 2) 左键点槽 1：起拖→松手按普通 PICKUP 并入（20+10=30 留槽，
        //    carried 归零）
        cs.mouse_down(18.0, 0.0, 0, false, false);
        cs.mouse_up(18.0, 0.0, 0, false);
        assert_eq!(cs.items[1], item(1, 30));
        assert!(cs.carried.is_empty());
        // 3) 双击槽 1（上一击也在槽 1）→ 先收空（carried 30），松手
        //    PICKUP_ALL 把其余同类全收拢
        cs.mouse_down(18.0, 0.0, 0, false, true);
        assert_eq!(cs.carried, item(1, 30));
        assert!(cs.items[1].is_empty());
        cs.mouse_up(18.0, 0.0, 0, false);
        assert_eq!(cs.carried, item(1, 60)); // 30 + 10(槽0空?) —— 槽0已空 + 30(槽4)
        assert_eq!(cs.items[2], item(2, 5)); // 异类不动
    }

    #[test]
    fn drag_charitable_splits_evenly() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 30), StackRef::default(), StackRef::default()]);
        // 收起 30
        cs.mouse_down(0.0, 0.0, 0, false, false);
        cs.mouse_up(0.0, 0.0, 0, false);
        // 起拖（有物在手按下）
        cs.mouse_down(0.0, 0.0, 0, false, false);
        cs.mouse_drag(18.0, 0.0, 0);
        cs.mouse_drag(36.0, 0.0, 0);
        cs.mouse_drag(54.0, 0.0, 0);
        cs.mouse_up(54.0, 0.0, 0, false);
        // 2 个可拖目标（结果槽不入集）：均分 15
        assert_eq!(cs.items[1], item(1, 15));
        assert_eq!(cs.items[2], item(1, 15));
        assert!(cs.items[3].is_empty()); // 结果槽
        assert!(cs.items[0].is_empty());
        assert!(cs.carried.is_empty()); // 30 = 2×15
    }

    #[test]
    fn drag_greedy_places_one_per_slot() {
        let mut cs = screen();
        cs.carried = item(1, 5);
        cs.mouse_down(0.0, 0.0, 1, false, false); // 右键起拖
        cs.mouse_drag(18.0, 0.0, 1);
        cs.mouse_drag(36.0, 0.0, 1);
        cs.mouse_up(36.0, 0.0, 1, false);
        assert_eq!(cs.items[1], item(1, 1));
        assert_eq!(cs.items[2], item(1, 1));
        assert_eq!(cs.carried, item(1, 3));
    }

    #[test]
    fn drag_single_slot_degenerates_to_pickup() {
        let mut cs = screen();
        cs.carried = item(1, 5);
        cs.mouse_down(0.0, 0.0, 0, false, false);
        cs.mouse_drag(18.0, 0.0, 0); // 只拖 1 格
        cs.mouse_up(18.0, 0.0, 0, false);
        // 单格 → 普通 PICKUP（button = 类型 0 = 全放，:362-367）
        assert_eq!(cs.items[1], item(1, 5));
        assert!(cs.carried.is_empty());
    }

    #[test]
    fn drag_cancel_on_different_button_release() {
        let mut cs = screen();
        cs.carried = item(1, 5);
        cs.mouse_down(0.0, 0.0, 0, false, false);
        cs.mouse_drag(18.0, 0.0, 0);
        // 右键松开 → 取消（:477-482）
        cs.mouse_up(18.0, 0.0, 1, false);
        assert!(cs.items[1].is_empty());
        assert_eq!(cs.carried, item(1, 5));
    }

    #[test]
    fn skip_next_release_guards_first_click() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 9), StackRef::default(), StackRef::default()]);
        // 按下（收起）→ 松手不二次生效
        cs.mouse_down(0.0, 0.0, 0, false, false);
        assert_eq!(cs.carried, item(1, 9));
        cs.mouse_up(0.0, 0.0, 0, false);
        assert_eq!(cs.carried, item(1, 9)); // skipNextRelease 吞掉
        assert!(cs.items[0].is_empty());
        // 再按再松 = 正常放回
        cs.mouse_down(0.0, 0.0, 0, false, false);
        assert!(cs.carried.is_empty());
        cs.mouse_up(0.0, 0.0, 0, false);
        assert_eq!(cs.items[0], item(1, 9));
    }

    #[test]
    fn hovered_and_outside_geometry() {
        let cs = screen();
        assert_eq!(cs.hovered_at(-1.0, 8.0), Some(0)); // 外扩 1
        assert_eq!(cs.hovered_at(15.9, 8.0), Some(0));
        assert_eq!(cs.hovered_at(18.0, 0.0), Some(1));
        assert_eq!(cs.hovered_at(0.0, 17.9), Some(4)); // 第二行
        assert_eq!(cs.hovered_at(0.0, 36.0), None); // 槽位命中盒外
        assert!(cs.clicked_outside(300.0, 300.0));
        assert!(!cs.clicked_outside(10.0, 10.0));
    }

    #[test]
    fn event_log_and_place_count_helpers() {
        let mut cs = screen();
        cs.set_items(vec![item(1, 9), StackRef::default(), StackRef::default()]);
        cs.mouse_down(0.0, 0.0, 0, true, false); // shift + 左键 = QUICK_MOVE
        let cmds = cs.take_cmds();
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            MenuCmd::Click {
                slot: 0,
                button: 0,
                input: Input::QuickMove
            }
        );
        // mask 编解码
        assert_eq!(get_quickcraft_header(get_quickcraft_mask(2, 1)), 2);
        assert_eq!(get_quickcraft_type(get_quickcraft_mask(2, 1)), 1);
        assert_eq!(quickcraft_place_count(3, QC_CHARITABLE, 30, 64), 10);
        assert_eq!(quickcraft_place_count(3, QC_GREEDY, 30, 64), 1);
        assert_eq!(quickcraft_place_count(3, QC_CLONE, 30, 64), 64);
    }

    #[test]
    fn quickcraft_preview_counts() {
        let mut cs = screen();
        cs.carried = item(1, 30);
        cs.mouse_down(0.0, 0.0, 0, false, false); // 起拖
        cs.mouse_drag(18.0, 0.0, 0);
        cs.mouse_drag(36.0, 0.0, 0);
        // 2 格均分：每格 15（空槽 carry 0）
        assert_eq!(cs.quickcraft_preview(1), Some(item(1, 15)));
        assert_eq!(cs.quickcraft_preview(2), Some(item(1, 15)));
        // 未入集的槽无预览
        assert_eq!(cs.quickcraft_preview(0), None);
        assert_eq!(cs.quickcraft_slots(), &[1, 2]);
    }

    #[test]
    fn carried_follows_cursor() {
        let mut cs = screen();
        cs.carried = item(1, 5);
        let (x, y, c) = cs.carried_draw(100.0, 50.0);
        assert_eq!((x, y), (92.0, 42.0)); // -8,-8
        assert_eq!(*c, item(1, 5));
    }
}

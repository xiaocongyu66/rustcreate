# KNOWN-DIVERGENCE（有意偏差登记簿）

本文件登记「与原版 26.1 行为有意不一致 / 尚未移植」的项，全部为**平台债或
裁剪债**，禁止无产品决策的私自修正（原版先行铁律）。代码内同 tag 检索
`KNOWN-DIVERGENCE`。本簿随修复波滚动更新。

## HAND-1 竖屏（aspect<1）第一人称手臂出画 = 原版行为，不修正

- 原版：手 pass 竖直 FOV 固定 70°（Camera.java:221-223 `calculateHudFov`
  = 70.0F）、投影 aspect = width/height（Projection.java:69-72
  `setPerspective(fov·π/180, width/height, …)`）——**没有任何 aspect<1
  适配**。竖屏（如 Android 竖屏 aspect≈0.45）时水平视场只剩桌面 16:9 的
  ~25%，renderPlayerArm 的摆位常数（ItemInHandRenderer.java:267-281，
  为桌面横屏设计）会把整条手臂推出画面：本机对原版矩阵链逐点复算，
  aspect 0.45 下臂 8 角 0/8 屏内（NDC x∈[1.51,4.56]）。
- 本仓：同原版（camera.rs `hud_proj` 竖直 70° + aspect=w/h；臂链全抄
  renderPlayerArm）。竖屏看不到手臂**不是 bug，是原版行为**——不发明
  水平 FOV 锁定/锚点平移等原版不存在的修正；若产品要求竖屏可见手持，
  需单独立项定夺（自定决策须走 porting-conventions 的偏差流程）。
- 关联：审计 129-E ③-2/④-C（/root/audit/129-E-hand.md）。

## HAND-2 手 pass 裁剪债（随 129-E 波登记，未移植）

按 26.1 原版机制列出、本仓未实现的项（修复波 133 覆盖矩阵链/bob/投影/
挥臂曲线四项，其余如下）：

1. 换持下放动画 inverseArmHeight（ItemInHandRenderer.java:270,344）与
   mainHandHeight 逐 tick 插值（:660-668 tick）——本仓 hand.rs 臂链按
   h=0 走，切槽瞬移。
2. (viewXRot−xBob)·0.1° / (viewYRot−yBob)·0.1° 视差补偿
   （renderHandsWithItems :358-361）——视角快速转动时手持微摆缺失。
3. bobHurt 受伤甩镜（GameRenderer.java:314-334）与死亡 FOV
   （Camera.java:225-240 modifyFovBasedOnDeathOrFluid）未移植——bob 门
   的 `!dying`（AbstractClientPlayer.java:84-87）与
   `!swimming`（本仓无游泳态）因此按可用状态近似。
4. 手 pass 光照硬编码 block_light=15/sky_light=15（gpu.rs draw_hand）——
   原版传玩家实体 packed light（ItemInHandRenderer.java:366-390 传入），
   洞穴/夜里本仓手臂恒全亮（审计 #9）。
5. 臂第二层 sleeve（AvatarRenderer.java:249-265 含可见性门）未画——
   手 pass 只取 player_index_slices[0][P_R_ARM] 基础层。
6. 使用类姿态 EAT/DRINK（:349-360）、BOW/CROSSBOW/TRIDENT 充能摆位
   （:472-577）、SHIELD BLOCK、BRUSH、单/双手地图（:205-254）未移植——
   进食/弓等走引擎自有观感。
7. 手持物为 GUI 精灵平片 quad + ICON_SIZE=0.45 自定（hand.rs）vs 原版
   builtin/generated 拉伸 3D 模型 + firstperson_righthand display 变换。

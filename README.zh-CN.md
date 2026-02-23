# KACF（Kinogram AutoCoding Framework）

语言： [English (Default)](README.md) | **简体中文-母语**


> 开发者是初三学生，一人团队，备战中考之余趁过年见缝插针开发此项目，觉得不错的话给个Star支持下呗😆非常感谢🙏🏻

> 由于开发者实在太忙了，以下~~部分~~是AI写的

> 使用自定义许可证禁止无授权商用是因为看到太多做的很好的开源软件被某些人拿去卖钱，希望至少不要发生在自己的项目身上😔

KACF 是一个面向真实开发场景的 AI 自动编程框架：你只需要描述目标，系统就会自动完成“写代码 -> 跑测试 -> 分析失败 -> 修复 -> 再验证”的闭环迭代。

对新手用户，KACF 的目标是：
- 真正零基础可上手，不要求编程经验
- 无需手写大量代码，通过自然语言目标驱动开发
- 失败后自动修复并继续推进，直到收敛或你主动停止

当前项目为 **Web UI 模式**。

## 为什么 KACF 强

- 自动编码闭环：模型输出补丁，系统自动应用并推进工程。
- 自动评测闭环：固定评测脚本驱动迭代，避免“只生成不验证”。
- 无人值守能力：支持自动迭代、自动恢复、轮次边界安全停止。
- 项目化工作流：多项目保存/加载/删除，按项目独立管理数据。
- 可观测性强：SSE 实时日志 + 指标 + 健康检查 + 调试日志接口。
- 语言包体系完整：前端文案统一语言包管理，启动时严格校验。

### 多用户版（`multi-user`）

- 三类角色：`Admin / User / Guest`
- 访客模式：无需注册即可进入，但全局只读、后端操作禁用
- 管理员面板：用户创建删除、封禁解封、强提醒、重置密码、审计日志
- 账户中心：修改昵称/密码/用户名/邮箱，切换登录方式
- 数据隔离：按用户独立 `ui_cache/projects/workspaces`
- 认证会话：基于 `kacf_session` Cookie

分支说明：
- `personal`：单用户版
- `multi-user`：多用户版（账户体系 + 管理员能力）

快速开始：

```bash
cargo build --release
cargo run --release
```

默认访问：`http://localhost:8080`

## 关键能力细节

### 自我迭代与无人值守

- 首轮可澄清，后续以补丁迭代为主
- 失败后按全局设置自动恢复并继续推进
- 到达“运行停止时间”后在轮次边界安全停止

### 项目与工作区管理

- 交互以“项目”为中心：保存/加载/删除/切换
- 每项目独立 workspace，避免相互污染
- 刷新页面后状态与配置可恢复

### 可观测性与稳定性

- 日志流 + SSE 断线回退轮询
- 运行态指标（含 readiness/gate）
- 前端离线锁与只读保护，避免误操作
- UI 缓冲与裁剪策略，降低长会话卡顿风险

## 目录结构（简）

```text
.
├── src/
├── static/
│   ├── index.html
│   ├── app.css
│   ├── js/
│   └── languages/
├── scripts/
└── autocoding_data/
```

## 常用命令

```bash
# 运行测试
bash scripts/run_tests.sh

# 发布检查
bash scripts/release_check.sh

# 指标门禁
bash scripts/metrics_gate.sh

# VM 压测与故障注入（需要先获取 kacf_session）
KACF_BASE_URL=http://127.0.0.1:8080 \
KACF_SESSION=<your_session_cookie_value> \
bash scripts/vm_ops_stress.sh --vm <vm_name> --rounds 10 --inject 50 --age 180
```

许可证：`LICENSE`（KACF Personal & Non-Commercial License 1.1，中英双语）

商业授权联系：`gregsons334@gmail.com`

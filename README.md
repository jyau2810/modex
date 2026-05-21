# Modex

Modex 是一个用于管理多个 Codex / ChatGPT 身份的 Tauri 桌面应用。它会为每个身份维护独立的 `CODEX_HOME`，切换当前生效的 Codex `auth.json`，通过 Codex app-server 协议读取配额信息，并在 macOS 菜单栏或 Windows 系统托盘中提供快捷操作。

## 产品功能

- 多身份管理：新增、导入、登录、删除 Codex 身份，并为每个身份维护独立的登录目录，避免不同账号的 `auth.json` 相互覆盖。
- API Key 身份：支持用 API Key 新增独立身份，自动读取账号名与额度，并可为该身份配置可选 Base URL。
- 一键切换账号：在主窗口或托盘 / 菜单栏中切换当前生效身份；切换时会同步目标身份的认证信息并启动 Codex。
- 配额状态查看：读取 Codex app-server 的配额数据，在账号列表中展示 5 小时和每周用量、重置时间、登录失效和配额受限状态。
- 快捷刷新与提醒：支持刷新单个或全部账号配额；当登录失效、配额刷新异常或额度恢复时，通过应用内日志和系统通知提示。
- 每日后台唤醒：可为团队版账号配置一个或多个定时唤醒时间、唤醒消息和配额保护阈值，在额度充足时自动执行轻量 Codex 调用。
- 唤醒保护机制：当 5 小时用量过高、本周剩余额度过低、登录不可用或非团队版账号时自动跳过唤醒；异常增长或执行超时会记录到日志并停止本轮唤醒。
- 运行日志面板：在主界面查看最近的唤醒、跳过、异常和客户端操作日志，便于追踪后台行为。
- 全局配置：可配置 Codex CLI 路径、Codex App 名称、Source Home 和配额轮询间隔，并可直接打开每个身份的配置目录。

## 技术栈

- 桌面外壳：Tauri 2
- 前端：React、TypeScript、Vite
- 样式：Tailwind CSS 和 Radix UI primitives
- 原生核心：Rust Tauri commands
- Codex 集成：已安装的 `codex` CLI、`CODEX_HOME` 和 `codex app-server --listen stdio://`

## 运行

安装前端依赖：

```bash
npm install
```

构建桌面应用前，还需要本机已安装 Rust 工具链，并且 `cargo` 可在 `PATH` 中访问。

启动开发模式：

```bash
npm run tauri dev
```

构建当前系统对应的应用：

```bash
./build.sh
```

运行当前系统已有的应用：

```bash
./app.sh
```

`app.sh` 只负责运行已经构建好的产物。如果当前系统对应的应用不存在，它会报错并提示先执行 `./build.sh`。

## 构建

构建前端：

```bash
npm run build
```

构建桌面应用：

```bash
./build.sh
```

macOS 构建产物会在 `build.sh` 末尾做本机 ad-hoc 签名，以保证系统通知等能力使用稳定的应用身份；这不是 Developer ID 签名或 notarization。Windows 可能会显示 SmartScreen 警告。

## 行为说明

启动时，Modex 会在后台刷新账号和配额信息，并显示不会阻塞主窗口的加载对话框。关闭主窗口后，应用仍可通过托盘或菜单栏继续使用。

账号数据沿用现有配置格式：

```text
~/Library/Application Support/Modex/config.json
%APPDATA%\Modex\config.json
```

托管身份目录继续使用：

```text
~/.modex/<12 digit id>
```

浏览器登录身份切换时会同步对应身份目录下的 Codex `auth.json`。API Key 身份会保存为独立身份，切换时同步 API Key 形式的 `auth.json`，并按需写入或清理该身份的 `openai_base_url` 配置；项目配置和本地会话不会被覆盖。

## 测试

前端测试：

```bash
npm test
```

Rust 测试：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Shell 打包测试：

```bash
python3 -m unittest tests/test_app_packaging.py
```

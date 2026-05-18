# Modex

Modex 是一个用于管理多个 Codex / ChatGPT 身份的 Tauri 桌面应用。它会为每个身份维护独立的 `CODEX_HOME`，切换当前生效的 Codex `auth.json`，通过 Codex app-server 协议读取配额信息，并在 macOS 菜单栏或 Windows 系统托盘中提供快捷操作。

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

切换账号只会替换当前生效的 Codex `auth.json`；项目配置和本地会话不会被覆盖。

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

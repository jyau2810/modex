# Modex

Modex 是一个本地 macOS 图形界面应用，用于管理多个 Codex ChatGPT 身份。

它会为每个身份保存独立登录态，可登录、切换当前 Codex 账号、检查配额，并在界面运行时监控配额恢复。切换账号时只更新主 Codex home 的 auth 文件，不覆盖项目配置和本地会话。

## 运行

打开已经打包好的本地应用：

```bash
./app.sh
```

如果 `dist/Modex.app` 已存在，`./app.sh` 会直接打开它，不需要安装 Python、Tk、uv 或其他构建环境。

## 构建

从源码重新构建本地应用：

```bash
MODEX_FORCE_BUILD=1 ./app.sh
```

应用使用 PyInstaller 打包，运行时不依赖系统 Python/Tk。仅构建时需要开发工具：

```bash
brew install uv
```

构建时，脚本会自动在仓库内创建并复用 `.venv-build`（默认 Python 3.12）并安装 `requirements-dev.txt`。

给其他机器使用时，分发构建后的 `dist/Modex.app` 即可；对方只运行 app，不需要安装构建工具。

如需手动指定构建 Python，可设置：

```bash
MODEX_PYTHON=/path/to/python3 ./app.sh
```

## 首次设置

首次启动时账号列表为空。点击 `新增账号` 后，Modex 会在 `~/.modex/<随机数>` 下创建独立账号目录并打开 Codex 登录；登录成功后，会根据登录凭据里的邮箱和计划类型自动命名。账号目录只保存该身份的认证信息，日常打开 Codex 仍使用主 Codex home。

账号加入列表后，可以在主面板中新增、删除账号，并修改是否监控配额。账号名称和目录由 Modex 自动管理；如需查看某个账号的本地配置，点击该行的 `配置目录`。

`设置` 中可修改 Codex 可执行文件路径和轮询间隔；保留为 `codex` 时会自动使用已安装 Codex App 内置的 CLI。

设置保存于：

```bash
~/Library/Application Support/Modex/config.json
```

## 开发

从源码运行：

```bash
python3 CodexAccountManager.py
```

运行测试：

```bash
python3 -m unittest discover tests
```

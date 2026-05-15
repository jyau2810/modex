# Modex

Modex 是一个本地 macOS 图形界面应用，用于管理多个 Codex ChatGPT 身份。

它会为每个身份保存独立登录态，可登录、切换当前 Codex 账号、检查配额，并在界面运行时监控配额恢复。切换账号时只更新主 Codex home 的 auth 文件，不覆盖项目配置和本地会话。

## 运行

构建并打开本地应用：

```bash
./app.sh
```

如需只构建本地应用：

```bash
python3 scripts/build_app.py --output-dir dist
```

应用使用 Tkinter。构建脚本会自动选择当前机器上可正常创建 Tk 窗口的 Python；如需手动指定，可设置 `MODEX_PYTHON=/path/to/python3`。

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

# TraeBuddy Keeper

<p align="center">
  <img src="src-tauri/icons/app-icon-source.png" width="128" alt="TraeBuddy Keeper 图标" />
</p>

TRAE / WorkBuddy 多账户管理与积分助手，基于 Tauri 2 的 Windows 桌面应用。

一个产品对应多个功能：账户管理、定时签到、积分查询、Token 续期、客户端切换，全部在一个本地应用里完成，数据只存在本机。

## 功能

### 账户管理
- **网页授权登录**：唯一添加账户的方式，走官方 OAuth 流程，支持 TRAE、WorkBuddy 国内版 / 国际版、CodeBuddy CN
- 多账户列表：头像、平台、签到状态、积分概览一目了然
- 账户编辑、删除、活跃账户切换（与客户端登录态联动）

### 签到与积分
- **一键签到 / 定时签到**：每日定时自动执行，支持 TRAE 与 WorkBuddy 两个平台范围（CodeBuddy CN 账号归入 WorkBuddy 范围）
- 签到健壮性：认证失败自动续期重试、限流阶梯退避、设备号轮换重试、账号间随机间隔、失败冷却与自动补签
- **真实积分数据**：TRAE 走官方积分包接口，WorkBuddy 直连腾讯 billing 拉取真实套餐，逐包展示剩余量与到期时间，查询结果缓存 60 秒
- **一键续期**：批量刷新全部账户凭证，延长登录态

### Token 统计
- 按时间范围统计 Token 用量，支持图表展示（recharts），结果缓存 10 分钟可手动强制刷新

### 客户端切换
- 在已安装的 TRAE / WorkBuddy（国内 / 国际）/ CodeBuddy CN 客户端之间切换登录身份：写回客户端配置、结束对应进程、恢复 vault 登录态
- 客户端路径支持注册表自动探测，设置页一键获取

### 其他
- **系统日志**：记录签到结果、积分查询失败、账户增删、Token 续期、授权登录等关键事件，保留时长可配置（默认 7 天），实时推送刷新
- **通知**：桌面 toast / 系统通知，模式可切换
- **托盘**：最小化到托盘常驻，托盘菜单支持立即签到
- **主题**：浅色 / 深色 / 跟随系统
- **备份与恢复**：一键导出数据库与凭证快照到任意文件夹，可整包导入恢复
- **自动更新**：基于 Tauri updater 的静默检查 + 手动安装，更新包经 minisign 签名校验

## 下载

前往 [Releases](https://github.com/Yqxnice/trae-buddy-keeper/releases) 下载最新版 Windows 安装包。

应用内置更新检查：启动 5 秒后静默检查新版本，发现新版仅 toast 提示，可在设置页「关于与更新」手动下载安装。

## 从源码构建

依赖：Node.js 18+、Rust（stable，MSVC 工具链）、Windows 10/11。

```bash
npm install
npm run tauri dev     # 开发调试
npm run tauri build   # 产物在 src-tauri/target/release/bundle/
```

如启用自动更新产物（`createUpdaterArtifacts`），构建前需设置签名私钥：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "C:\Users\<you>\.tauri\<your-app>.key"
npm run tauri build
```

CI 发版：推送 `v*` 标签触发 GitHub Actions 自动构建并发布 Release，签名私钥通过仓库 Secret `TAURI_SIGNING_PRIVATE_KEY` 提供。

## 数据存储

所有数据（SQLite 数据库、账户凭证 vault）存储于：

```
%APPDATA%\com.traebuddy.keeper
```

设置页「关于与更新」可一键打开数据文件夹。

## 技术栈

- **前端**：React 18 + TypeScript + Zustand + recharts + Tailwind CSS
- **后端**：Rust + Tauri 2 + rusqlite + axum（OAuth 回调）+ reqwest
- **发布**：GitHub Actions + Tauri updater（minisign 签名）

## 隐私说明

- **不上传任何用户信息**：应用没有自有服务器，所有数据（账户、凭证、日志、积分记录）仅保存在本机 `%APPDATA%\com.traebuddy.keeper`，除你本人外无人能获取
- 网络请求只发往两类目标：
  - **官方接口**：登录授权、签到、积分查询、Token 续期——与官方客户端发出的请求同源，凭证明文仅在本机数据库与 vault 中使用
  - **GitHub Releases**：仅用于检查与下载应用更新
- 备份文件由你自行选择保存位置，应用不会将其发送到任何地方

## 免责声明

- 本项目为个人效率工具，与 TRAE、WorkBuddy、CodeBuddy 官方无关
- 签到与积分查询通过官方客户端同源接口实现，**存在被官方风控判定为异常行为进而限制或封禁账号的可能**。项目内置了随机请求间隔、限流退避、设备号一致性等缓解措施，但无法保证规避所有风控，**使用产生的账号风险由你自行承担**
- 请合理控制账号数量与签到频率，勿将本工具用于批量注册、多开滥用等场景

## 许可证

[MIT](LICENSE)

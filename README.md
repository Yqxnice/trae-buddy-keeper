# TraeBuddy Keeper

<p align="center">
  <img src="src-tauri/icons/app-icon-source.png" width="128" alt="TraeBuddy Keeper 图标" />
</p>

TRAE / WorkBuddy / Qoder CN 多账户管理与积分助手，基于 Tauri 2 的 Windows 桌面应用。

一个产品对应多个功能：账户管理、定时签到、积分查询、Token 续期与用量统计、客户端切换，全部在一个本地应用里完成，数据只存在本机。

## 功能

### 账户管理
- **网页授权登录**：唯一添加账户的方式，走官方 OAuth / 设备流流程，支持 TRAE、WorkBuddy 国内版 / 国际版、CodeBuddy CN、Qoder CN
- 多账户列表：头像、平台、签到状态、积分概览一目了然
- 账户编辑、删除、活跃账户切换（与客户端登录态联动）
- **登录档案按平台分目录**：`vaults/trae/<账户UUID>`（TRAE）与 `vaults/qoder_cn/<账户UUID>`（Qoder CN），一眼可辨；旧版 TRAE 档案散落在 `vaults/` 根目录，首次访问时自动搬移
- **删除账户会同步清理该账户的客户端登录档案**（Trae `vaults/trae/<id>` 与 Qoder CN `vaults/qoder_cn/<id>`），避免残留可解密的登录态密文与孤儿目录；删除前弹窗确认，默认把档案轮转为 `<id>.bak` 保留（可手动恢复），取消勾选则彻底删除。全局的 Qoder CN `last` 槽不在此列

### 签到与积分
- **一键签到 / 定时签到**：每日定时自动执行，可选 TRAE / WorkBuddy / Qoder CN 三个平台范围（CodeBuddy CN 账号归入 WorkBuddy 范围）
- 签到健壮性：认证失败自动续期重试、限流阶梯退避、设备号轮换重试、账号间随机间隔、失败冷却与自动补签
- **Qoder CN 活动领取**：拉取官方活动列表后逐个幂等领取，响应丢失时复查列表确认，避免重复领取；授权后回查真实领取状态。无可领活动时区分「今日已领」「账号未获活动投放」「暂不可领」「状态解析未识别」，不再一律报「没有活动」
- **每账号设备身份**：TRAE 签到按账号独立 `x-device-id`（9074 被拉黑时换号重试）；Qoder CN 按账号绑定独立稳定的 `Cosy-MachineId`，并透传本机客户端原生的 `Cosy-MachineToken` / `Cosy-MachineCode` / `Cosy-MachineType`（读不到就不伪造），一次绑定后不轮换。Qoder CN 活动列表由服务端按账号维度下发：账号未获投放时列表里只有展示型活动，此时按「账号暂无活动资格」提示并记为跳过（不累计失败、不进冷却），与设备指纹无关，重试或更换设备头都不会产生可领活动
- **真实积分数据**：TRAE 走官方积分包接口，WorkBuddy 直连腾讯 billing 拉取真实套餐，Qoder CN 走官方额度接口并叠加官网逐包明细，逐包展示剩余量与到期时间，查询结果缓存 60 秒
- **手动续期（行内 / 一键批量）**：不受 24 小时到期窗口限制，任意账号可随时刷新凭证，仅保留单账号 5 分钟限频防误触；自动巡检仍只处理 24 小时内到期（含已过期）的账号。批量续期会逐个刷新全部账户凭证（Qoder CN 按刷新令牌前缀走设备令牌 / 作业令牌端点）

### Token 统计
- 按时间范围统计 Token 用量，支持图表展示（recharts），结果缓存 10 分钟可手动强制刷新
- 本地会话日志按产品隔离：WorkBuddy 国内 / 国际、CodeBuddy IDE、Qoder CN；TRAE 走云端会话用量接口

### 客户端切换
- 在已安装的 TRAE / WorkBuddy（国内 / 国际）/ CodeBuddy CN / Qoder CN 客户端之间切换登录身份：写回客户端配置、结束对应进程、恢复登录态档案
- **Qoder CN 客户端路径只认本体**：自动探测按卸载项名称排除 Qoder IDE / Qoder Work 等其它产品线，并从 `uninstallerIcon.ico` 所在目录推出主程序（`Qoder CN.exe` / `Qoder CN IDE.exe`）；历史误填到设置里的其它 Qoder 路径会在打开设置页时自动纠正，进程检测与切换不会再指向错误客户端
- 客户端路径支持注册表自动探测，设置页一键获取
- **Qoder CN 两代布局自动识别**：数据目录从安装目录 `resources/product.json#dataDirectoryName` 读取（当前桌面 App 为 `%APPDATA%\com.qodercn.app.stable`，旧版 IDE fork 为 `%APPDATA%\Qoder[CN]`），避免把登录态写进客户端根本不读的目录。桌面 App 按 `auth.v1.dat`（`v10` + AES-256-GCM，密钥来自 `Local State` 中 DPAPI 保护的字段）、`auth-profile-overlays.v1.dat`、`auth.machine-id`、`Local State`、`Preferences`、`Local Storage/leveldb` 等载体冷切换；`main.sqlite` 是业务库（会话 / 设置），不随账号搬运。旧版布局仍按 `state.vscdb` 的 15 项白名单兼容，恢复前清除 `code.lock` 与 WAL / SHM
- **Qoder CN 启动后确定性身份核验**：只读解密 `auth.v1.dat` 取出真实 uid 与目标账号比对（凭据不落库、不写日志），连续 4 帧一致才确认；**只有确认登录到目标账号才算成功**，读不出身份、身份不符或凭据被清除一律返回警示，且不把「当前客户端账号」标记指向目标。切换前把静止现场存成持久 `last` 槽并在登录态仍在时回写当前账号档案（覆盖前轮转 `.bak`，主档损坏可用 `.bak` 恢复），客户端保持现状**不硬回滚**，也可用「恢复切换前现场」人工回退
- **档案与客户端布局绑定**：按旧版 IDE 目录建立的 Qoder CN 档案不能用于桌面 App 布局，切换时会明确提示「档案与当前客户端不匹配」，需在客户端登录该账号后重新「存档」
- **TRAE 按 15 项白名单整目录冷切换**：建档前需完全退出客户端；切换时先暂存当前现场，恢复前清除 `code.lock` 与 `state.vscdb-wal/-shm`（避免回放上一账号事务），并对 vault 缺失的登录态载体做对称删除；`Local State` / `machineid` 等设备绑定文件始终保留
- **切换前会话预检**：用应用内保存的 access_token 打一次官方签到状态接口（5 秒超时），仅当明确返回 401 / `code=1001` 时给出警示；网络异常一律放行，且**判死也照常切换**——切进该账号重新登录是会话被吊销时唯一的恢复手段
- **不确认 ≠ 成功**：TRAE 启动后 25 秒守护、Qoder CN 启动后 20 秒身份核验，未能确认登录时客户端保持现状不硬回滚，档案被标记「待验证」（列表行内可见），并引导重新登录 + 重新建档；只有客户端进程根本没起来才回滚并重启原账号。TRAE 成功确认时自动把轮换后的 `storage.json` 回写档案保鲜

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
├── trae-buddy-keeper.db      # 数据库（账户、设置、签到记录）
├── vaults\
│   ├── trae\<账户UUID>       # TRAE 客户端登录态快照（每账号一个目录）
│   └── qoder_cn\             # Qoder CN 快照；last / last.bak 为全局切换前现场
└── logs\
```

设置页「关于与更新」可一键打开数据文件夹。旧版把 TRAE 档案直接放在 `vaults\<账户UUID>`，升级后首次访问任意 TRAE 档案会自动搬到 `vaults\trae\`，无需手动操作。

## 技术栈

- **前端**：React 18 + TypeScript + Vite + Zustand + recharts + lucide-react；样式为原生 CSS（`src/styles/tokens.css` 设计令牌 + 按组件拆分的 CSS），未使用 Tailwind 等 UI 框架
- **后端**：Rust + Tauri 2 + rusqlite（bundled SQLite）+ tokio + reqwest + axum（OAuth 本地回调）
- **发布**：GitHub Actions + Tauri updater（minisign 签名）

## 隐私说明

- **不上传任何用户信息**：应用没有自有服务器，所有数据（账户、凭证、日志、积分记录）仅保存在本机 `%APPDATA%\com.traebuddy.keeper`，除你本人外无人能获取
- **凭证静态加密**：`accounts` 表的 `access_token` / `refresh_token` / `cookies` / `device_key` 以 Windows DPAPI 加密后落库（密文绑定当前 Windows 用户，其他用户或设备无法解密），格式 `dpapi:v1:<base64>`；旧版本留下的明文在首次启动时自动迁移。空值仍为空，不影响「缺少 token」判定
- 网络请求只发往两类目标：
  - **官方接口**：登录授权、签到、积分查询、Token 续期——与官方客户端发出的请求同源，凭证仅在进程内存中解密后使用
  - **GitHub Releases**：仅用于检查与下载应用更新
- 备份文件由你自行选择保存位置，应用不会将其发送到任何地方
- **备份与恢复的边界**：备份中的凭证是 DPAPI 密文，**只能在同一个 Windows 用户下恢复**。跨用户 / 跨设备导入时这些列无法解密，应用会把它们按空凭证处理并在系统日志中告警，相关账户需重新网页授权登录；`vaults/` 客户端登录态快照仍是明文文件，请自行控制备份目录的访问权限

## 免责声明

- 本项目为个人效率工具，与 TRAE、WorkBuddy、CodeBuddy 官方无关
- 签到与积分查询通过官方客户端同源接口实现，**存在被官方风控判定为异常行为进而限制或封禁账号的可能**。项目内置了随机请求间隔、限流退避、设备号一致性等缓解措施，但无法保证规避所有风控，**使用产生的账号风险由你自行承担**
- 请合理控制账号数量与签到频率，勿将本工具用于批量注册、多开滥用等场景

## 许可证

[MIT](LICENSE)

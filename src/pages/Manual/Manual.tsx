import type { ReactNode } from "react";
import { Globe, CalendarCheck, Coins } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import "./Manual.css";

/** 手册条目：名称 + 说明 */
type Row = { name: string; desc: ReactNode };
type Section = { title: string; rows: Row[] };

const SECTIONS: Section[] = [
  {
    title: "界面与导航",
    rows: [
      {
        name: "产品切换",
        desc: "标题栏可切换 TRAE / WorkBuddy 两个产品，侧边栏各页面只展示当前产品的账号与数据",
      },
      {
        name: "状态点",
        desc: "侧边栏「账号」出现橙色呼吸灯，表示有账号签到失败、冷却中或凭证过期；「签到」出现绿色呼吸灯，表示有账号今日尚未签到；全部健康时状态点隐藏。状态点约 60 秒刷新一次",
      },
      {
        name: "常驻托盘",
        desc: "点击窗口关闭按钮只是隐藏到托盘，应用仍在后台运行；托盘菜单支持「显示主窗口 / 一键签到 / 退出」，其中一键签到会签到所有产品的账号",
      },
    ],
  },
  {
    title: "账号管理",
    rows: [
      {
        name: "添加账号",
        desc: "唯一添加方式为网页授权登录：TRAE 走官方 OAuth 流程；WorkBuddy 登录时需选择国内版或国际版；CodeBuddy CN 与 WorkBuddy 国内版同账号体系，登录 WorkBuddy 后即可切换到 CodeBuddy CN 客户端",
      },
      {
        name: "授权流程",
        desc: "点击授权按钮后自动打开系统浏览器授权页，完成登录后凭证自动捕获入库。若自动回调失败，可将浏览器地址栏中的回调 URL 粘贴到弹窗输入框手动完成登录",
      },
      {
        name: "编辑与删除",
        desc: "账号行末「⋮」菜单：编辑支持修改名称、邮箱、accessToken、refreshToken；删除账户会同时清除其签到记录",
      },
      {
        name: "存档（仅 TRAE）",
        desc: "把当前 Trae 客户端的登录态快照到选中账号，作为后续「切换客户端」的前提；存档前需先关闭 Trae 客户端",
      },
      {
        name: "客户端切换",
        desc: "账号行内的客户端图标即切换入口，当前登录的客户端会高亮显示。切换会把该账号的登录态写回客户端配置并重启客户端；TRAE 产品下切换 Trae，WorkBuddy 产品下可在 WorkBuddy 与 CodeBuddy CN 之间切换",
      },
      {
        name: "国际版说明",
        desc: "WorkBuddy 国际版账号（列表中标注「国际版」）没有签到接口，签到按钮禁用，仅支持切换客户端与用量统计",
      },
    ],
  },
  {
    title: "签到",
    rows: [
      {
        name: "签到日界",
        desc: "每天 00:00 为签到日界，0 点后签到记为当日完成；已签账号次日 0 点后可再次签到",
      },
      {
        name: "手动签到",
        desc: "「签到」页支持全部账号或手动勾选执行，可跳过已签与失效账号，逐账号展示结果；「账号」页一键签到批量执行全部可签账号；托盘「一键签到」覆盖所有产品",
      },
      {
        name: "定时签到",
        desc: "「定时任务」页开启每日自动签到并设置执行时间；平台范围在「设置 → 定时签到」配置，WorkBuddy 范围包含 CodeBuddy CN 账号",
      },
      {
        name: "失败处理",
        desc: "批量签到时账号之间按设定间隔执行，连续失败达到最大次数的账号进入冷却，冷却期间跳过签到；冷却结束后由超时补签自动重试",
      },
      {
        name: "超时补签",
        desc: "到达当日设定时间后每 10 分钟巡检一次，应用启动时若已过设定时间也会补签一次；对当日未签到、不在冷却且凭证有效的账号自动补签，当日已签成功的账号不会再发起请求",
      },
      {
        name: "签到健壮性",
        desc: "认证失败自动续期并重试一次；限流与服务端错误按阶梯退避重试并轮换设备号；多账号之间加入随机间隔降低风控风险",
      },
    ],
  },
  {
    title: "积分",
    rows: [
      {
        name: "数据来源",
        desc: "TRAE 走官方积分权益接口；WorkBuddy（含 CodeBuddy CN）直连腾讯 billing 拉取真实套餐数据，均为账号真实积分",
      },
      {
        name: "查询与缓存",
        desc: "查询结果缓存 60 秒，签到成功后自动刷新积分；「积分」页与「账号」页均可手动刷新，立即重新查询",
      },
      {
        name: "积分包明细",
        desc: "逐积分包展示剩余量与到期时间；「总览」页会提醒 7 日内到期的积分包，到期前请及时使用",
      },
    ],
  },
  {
    title: "Token 续期与统计",
    rows: [
      {
        name: "续期窗口",
        desc: "Token 到期前 24 小时内（或已过期）才可续期，其余时间续期按钮禁用；已过期的账号续期后即可恢复正常使用",
      },
      {
        name: "自动巡检",
        desc: "应用启动时与之后每 6 小时自动续期临期（24 小时内到期）账号；「账号」页一键续期可手动批量触发",
      },
      {
        name: "Token 统计",
        desc: "按数据来源（WorkBuddy / WorkBuddy 国际版 / CodeBuddy IDE / TRAE 云端）与时间范围统计 Token 用量；结果缓存 10 分钟，手动刷新会强制重算",
      },
    ],
  },
  {
    title: "系统日志",
    rows: [
      {
        name: "记录范围",
        desc: "签到结果、积分查询失败、账户增删、Token 续期、网页授权登录等关键事件均会记录",
      },
      {
        name: "查看与清理",
        desc: "默认展示最近 300 条，支持按级别筛选；保留时长在「设置 → 日志保留天数」配置（默认 7 天，超出自动清理）；日志写入后实时推送刷新，无需手动刷新页面",
      },
    ],
  },
  {
    title: "设置",
    rows: [
      {
        name: "外观",
        desc: "浅色 / 深色 / 跟随系统，切换立即生效并自动保存；日志保留天数是唯一需要点击「保存」生效的设置项",
      },
      { name: "通知", desc: "签到完成、续期等事件的通知方式：应用内 Toast / 系统通知 / 两者 / 不通知" },
      { name: "启动", desc: "开机自动启动；启动时最小化到托盘（不显示主窗口）" },
      {
        name: "客户端路径",
        desc: "TRAE / WorkBuddy / WorkBuddy AI / CodeBuddy CN 四个客户端的安装路径，可点击「自动获取」从注册表探测，未检测到即视为未安装；客户端切换功能依赖正确的路径配置",
      },
      {
        name: "备份与恢复",
        desc: "「备份数据」将数据库与授权会话导出到你选择的文件夹；「导入数据」从备份文件夹恢复并覆盖现有数据，完成后建议重启应用",
      },
      {
        name: "关于与更新",
        desc: "展示存储位置并可一键打开数据文件夹；应用启动 5 秒后自动静默检查更新，发现新版本仅弹提示不打扰使用，可在本页手动下载安装并重启生效",
      },
    ],
  },
  {
    title: "数据与隐私",
    rows: [
      {
        name: "本地存储",
        desc: "全部数据（数据库、凭证 vault、日志）仅存储在本机 %APPDATA%\\com.traebuddy.keeper，应用没有自有服务器",
      },
      {
        name: "网络请求",
        desc: "仅发往两类目标：官方接口（登录授权、签到、积分、续期）与 GitHub Releases（检查和下载更新），不上传任何用户信息",
      },
    ],
  },
];

const FAQ: { q: string; a: ReactNode }[] = [
  {
    q: "网页授权一直等待，没有反应？",
    a: "常见于本地回调端口（18923）被占用或被代理拦截：先取消授权后重试；若系统开启了本地代理，请将 127.0.0.1 加入代理绕过列表。也可以把浏览器授权后地址栏中的回调 URL 粘贴到弹窗输入框手动完成登录。",
  },
  {
    q: "签到提示认证失败？",
    a: "多为登录凭证过期：先尝试对该账号「续期」；若续期也失败，删除账号后重新网页授权即可。",
  },
  {
    q: "签到按钮为什么被禁用 / 签到被跳过？",
    a: "四种情况：今日已签（次日 00:00 后可再签）、冷却中、凭证已失效、WorkBuddy 国际版（无签到接口）。将鼠标悬停在按钮上可查看具体原因。",
  },
  {
    q: "为什么 Token 不能续期？",
    a: "剩余有效期超过 24 小时的账号暂不需要续期，按钮会禁用；到期前 24 小时内或已过期时才可续期。",
  },
  {
    q: "冷却中的账号会自动恢复吗？",
    a: "会。冷却结束后由超时补签（每 10 分钟一次）自动重试签到，无需手动干预；也可在「签到」页手动立即重试。",
  },
  {
    q: "换电脑或重装系统如何迁移数据？",
    a: "旧机器上「设置 → 备份数据」导出到 U 盘或网盘，新机器安装本应用后「导入数据」选择该备份文件夹，完成后重启应用。",
  },
  {
    q: "发现新版本后如何升级？",
    a: "应用启动后会自动检查更新并弹提示；进入「设置 → 关于与更新」点击「下载并安装」，下载完成后点击「重启应用」即完成升级。",
  },
];

/** 使用手册：应用内自述文档，覆盖从添加账号到备份恢复的完整使用流程 */
export function ManualPage() {
  return (
    <div className="manual">
      <PageHeader title="使用手册" sub="从添加账号到备份恢复的完整使用说明" />

      {/* 快速上手三步 */}
      <div className="m-steps">
        <div className="m-step">
          <span className="m-no">1</span>
          <Globe size={16} strokeWidth={1.7} className="m-ico" />
          <div>
            <div className="st-name">添加账号</div>
            <div className="st-desc">
              「账号」页点击「网页授权」，在浏览器完成登录后凭证自动入库
            </div>
          </div>
        </div>
        <span className="divider-v" />
        <div className="m-step">
          <span className="m-no">2</span>
          <CalendarCheck size={16} strokeWidth={1.7} className="m-ico" />
          <div>
            <div className="st-name">执行签到</div>
            <div className="st-desc">
              「签到」页手动执行，或在「定时任务」开启每日自动签到与补签
            </div>
          </div>
        </div>
        <span className="divider-v" />
        <div className="m-step">
          <span className="m-no">3</span>
          <Coins size={16} strokeWidth={1.7} className="m-ico" />
          <div>
            <div className="st-name">管理积分</div>
            <div className="st-desc">
              「积分」页查看各账号积分包余额与到期，「总览」页看每日概况
            </div>
          </div>
        </div>
      </div>

      {/* 分节说明 */}
      {SECTIONS.map((sec) => (
        <div key={sec.title}>
          <div className="gtitle">{sec.title}</div>
          <div className="m-card">
            {sec.rows.map((row) => (
              <div className="m-row" key={row.name}>
                <div className="m-name">{row.name}</div>
                <div className="m-desc">{row.desc}</div>
              </div>
            ))}
          </div>
        </div>
      ))}

      {/* 常见问题 */}
      <div className="gtitle">常见问题</div>
      <div className="m-card">
        {FAQ.map((f, i) => (
          <details className="m-faq" key={i} open={i === 0}>
            <summary>{f.q}</summary>
            <p className="m-faq-a">{f.a}</p>
          </details>
        ))}
      </div>

      {/* 静默信息条 */}
      <div className="quiet">
        <span>
          本手册与当前版本功能对应 · 数据仅存本地 · 签到通过官方同源接口实现，请注意账号风险
        </span>
      </div>
    </div>
  );
}

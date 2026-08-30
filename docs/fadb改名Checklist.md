# fadb 改名与发布 Checklist

> 项目更名为 **fadb** — a featherweight ADB toolbox, in Rust.

## 一、先锁定资产（半小时内搞定，防抢注）

- [ ] 注册域名 **fadb.dev**（可选 + fadb.rs）
- [ ] 在 crates.io 上占位：`cargo new fadb && cargo publish`（发布一个 0.0.0-placeholder 版本，描述写清楚，避免被抢注——**这步很关键，crate 名先到先得**）
- [ ] npm 占位（可选）：`npm publish` 一个空 `fadb` 包
- [ ] GitHub 上注册组织 `fadb-dev` 之类备用（`fadb` 用户名已被占，确认下它是不是僵尸号，是的话可以试着向 GitHub 申诉释放，但别抱希望）

## 二、GitHub 仓库改名

- [ ] Settings → Rename 仓库为 `fadb`（旧链接自动 301 跳转，star / issue / PR / fork 全部保留）
- [ ] 改仓库 description 和 topics（加上 `adb` `android` `rust` `egui` `devtools`）
- [ ] 本地 remote 更新：`git remote set-url origin git@github.com:yeqing17/fadb.git`（不改也能用，跳转有效，但建议改）

## 三、代码层改名（工作量主要在这）

- [ ] `Cargo.toml` workspace：crate 名改为 `fadb-desktop`（或 `fadb-gui`），**目录名一并改**
- [ ] `cargo fmt / clippy / test / build` 四条命令全跑一遍，确认 workspace 改名后无残留引用
- [ ] 全局搜索替换（注意区分大小写）：
  - 大驼峰显示名 → `Fadb`（UI 显示名、窗口标题）
  - 小写标识符 / 路径 → `fadb`
  - 环境变量 → `FADB_FAKE`（README 里也要同步改）
- [ ] 检查 `docs/clean-room.md`、`docs/feature-matrix.md` 等文档里的项目名引用
- [ ] 配置文件 / 缓存目录名：检查代码里有没有往 `~/.config/xxx` 这类路径写东西；改名意味着用户旧配置会"丢失"，要么做迁移逻辑，要么在 release note 里说明

## 四、README 和门面（决定爆款相的部分）

- [ ] 标题换成 `fadb`，副标题上 slogan：**a featherweight ADB toolbox, in Rust**
- [ ] 顶部加 badges：crates.io version、license、CI status、downloads
- [ ] **补一张好看的截图或 GIF 放最顶上**——GUI 工具没有 demo 图，star 转化率差一个数量级
- [ ] 安装方式加上 `cargo install fadb`（等正式发布后）
- [ ] 清理 README 里所有旧项目名的历史描述

## 五、发布与推广（改完名才是开始）

- [ ] 打一个 **v0.8.0**（改名本身就值得一个 minor version），release note 里写明已完成更名
- [ ] `cargo publish` 正式版
- [ ] 发帖渠道按效果排序：
  1. **r/rust** 的 "What's everyone working on" 帖或直接发 showcase（GUI 工具带截图在 r/rust 很吃香）
  2. **This Week in Rust** 提交
  3. V2EX / 掘金 / 少数派（中文圈）
  4. X / 即刻，带 #rustlang #androiddev 标签
- [ ] 提交到 awesome-rust、awesome-adb 这类列表（提 PR 即可，免费流量）

## 两个坑提前说

1. **改名后 24h 内别大规模宣传**——GitHub 跳转缓存、crates.io 索引都有延迟，等链接全部生效再发
2. **改名和发版分开做 commit**——万一改名引入 bug，方便 bisect 定位

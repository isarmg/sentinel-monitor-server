# Rust 安全与工程边界

本仓库生产源码、构建脚本和 Rust 测试没有 `unsafe` 块、`unsafe fn` 或 `unsafe impl`。根 `Cargo.toml` 使用 `unsafe_code = "forbid"`，后续局部 `allow` 不能绕过。原有 libc 文件 flags 常量改为 rustix 的等值安全 flags，移除直接 libc 依赖；日志 subscriber 的实际运行依赖保留。

这不表示 xcss、SQLx、密码库或其他第三方库没有原生实现；本记录的逐项责任范围是本仓库自有源码。

本项目保持单 Rust 包布局及唯一根锁文件。`src/main.rs` 组合模块，`cli` 处理命令，`app` 组装服务，`sqlite/` 处理连接、初始化、租约及路径，`routes/` 处理 HTTP 业务。仓库根目录职责为：

| 目录 | 责任 |
| --- | --- |
| `web/` | 嵌入 Server 的浏览器应用 |
| `config/` | JSON/环境配置示例及固定 MediaMTX 合同 |
| `deploy/` | 生命周期脚本、systemd 与反向代理示例 |
| `scripts/` | 构建、打包、检查和验收入口 |
| `schema/` | 当前 SQLite 定义及生成结果 |
| `docs/` | 使用、架构和验收说明 |

发行树中的生命周期材料也使用 `deploy/`，Rust 和 Shell 的固定清单同步校验该布局。构建和测试脚本不进入用户运行闭包。旧发行文档保持其原版本路径，不作为当前推荐入口。

Server 只消费 Client 上报的身份、三态能力和健康状态。模拟协议/浏览器 fixture、真实 MediaMTX 与真实摄像头是不同证据；本次没有真实摄像头可用，不扩大厂商、型号或固件支持声明。Linux AMD64 的生命周期、重定位和正式制品验证仍由对应入口执行，macOS 格式检查不能代替这些结果。

## 规范适用与本轮证据

| 条款 | 当前实现与验证边界 |
| --- | --- |
| 3、5 | xcss 1.0.0 固定官方完整源码修订，manifest 与唯一 Cargo.lock 同步；官方输入策略需按该锁验证，Web 使用正式资产的真实 URL 与 SRI。 |
| 4.1、4.2 | 单 Rust 包、明确领域模块、web/config/deploy/scripts 职责；同步构建脚本、CI、文档和发行清单，不改历史标签/验收记录。 |
| 7、8、9 | 正常服务保持严格当前结构、显式 init 与只读配置校验；服务身份和私有数据属主限制保留。 |
| 用户 unsafe 审核要求、8、21 | 自有 unsafe 为零且整包 forbid；依赖的必要原生边界由固定 xcss 和各依赖负责，不将静态检查写成 Linux 运行证据。 |
| 19–23、25 | Server 源码与当前文档不包含离线辅助工具协议；签名、Linux 运行、最终制品及设备证据分别验证。 |

本轮本地证据：Rust 格式检查及自有 unsafe 扫描；Linux 目标交叉静态验证按实际结果记录。macOS 本机没有执行 Linux 原生锁、生命周期或签名制品运行，正式 CI 的对应检查完成后才作为发行证据。

10 个当前 Shell 文件的语法、真实前端 TypeScript/Vite 构建及 7 项前端单元测试已通过；没有真实摄像头或真实设备状态证据。

此前分模块发布的 xcss 1.0.0 Web 制品曾从真实 npm 缓存逐字节复核（历史验收；当前已改为一个 @xcss/web 单体，须重新验收）：SHA-512 与新 npm 锁一致，SHA-256 与官方 GitHub release 资产 digest 一致，包内版本均为 1.0.0；axe 浏览器验收依赖锁为 4.13.0。

最终正式 Web 输入的 xcss/fonts 检查、strict TypeScript、Vite 构建与 7 项前端单测已通过；Linux AMD64 目标全部 targets/features 的 Clippy `-D warnings` 再次通过。Linux 生命周期、重定位及最终包仍由原生 CI 和发行流水线验收。

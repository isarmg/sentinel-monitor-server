# xcos 完整功能与取舍清单

本文按当前 `1.0.0` 工作树逐项盘点 xcos 的真实能力、保证、交付工具和明确边界。代码、
`schema/generated/current_schema.sql`、`web/src/protocol-contract.json`、`config/mediamtx.lock` 与发行 manifest 是
实现依据。本文供设计和代码评审使用；日常任务从[文档入口](README.md)进入。

本清单帮助开发人员回答四个问题：某段代码保护什么；删除后哪条用户旅程或安全不变量会消失；删除
需要同时清理哪些消费者；变更完成至少要取得什么证据。

## 1. 阅读规则

### 1.1 分类

| 分类 | 判断标准 |
|---|---|
| 核心 | 直接构成浏览器摄像头监控、录像回放或设备管理主目标，删除后产品定位改变 |
| 保障 | 用户未必直接看到，但负责身份、机密性、一致性、资源边界或失败关闭 |
| 可选 | 只服务部分设备或部署，可在接受明确功能损失后删除 |
| 建议保留 | 不决定产品存在，但显著改善可用性、诊断或操作闭环 |
| 开发运维 | 构建、验证、安装、诊断、发布、文档和供应链能力 |

### 1.2 复杂度

| 复杂度 | 判断标准 |
|---|---|
| 低 | 单个独立入口或小范围配置，通常不改持久状态和跨组件协议 |
| 中 | 跨两个以上模块、前后端或脚本，需要成组删除和回归验证 |
| 高 | 跨协议、Schema、密码学、外部系统、持久状态或发行闭包，不能只删按钮或路由 |

### 1.3 身份边界

Xcos 控制面只有 Administrator 一种身份。`_common_administrators` 表没有 `role` 列，登录成功的 wire response 固定
`role:"admin"`，账户名称为 canonical `username`，业务外键引用不透明的 `administrator_id`。
xcss 提供管理员创建、凭据管理与停用能力。摄像头 RTSP/ONVIF 凭据只保存在 Client；实例授权码和
媒体 JWT `actions` 属于数据面授权，不代表控制面角色。相同 username 文本不会把摄像头身份与 Administrator 关联。

### 1.4 删除闭包

删除任一功能时至少检查：Rust 路由与 DTO、SQLite DDL/查询、React 页面与运行时 guard、MediaMTX 配置、
原生生命周期脚本、配置样例、发行 manifest、正反测试和本套中文文档。隐藏 React 按钮不等于删除功能；
只删数据库列也不等于删除协议。

## 2. 总体定位、平台与架构

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-P-001 | 浏览器摄像头监控产品：Rust 控制面管理摄像头和授权，MediaMTX 承担 RTSP、WHEP、HLS 与录像 | `src/routes/`、`src/mediamtx.rs`、`config/mediamtx.yml` | 核心 | 高 | 删除任一主组件都会失去控制面或媒体面，项目不再完整 | 摄像头添加、直播、录像、重启链路 |
| SEN-P-002 | Server 开发、测试、正式编译目标唯一为 `x86_64-unknown-linux-gnu` | `xcss::server_target`、`build.rs`、`rust-toolchain.toml` | 保障 | 高 | 放宽后会产生未经验证的平台二进制，且可能与 companion 平台失配 | 非目标编译必须失败；目标常量与 Cargo target 一致 |
| SEN-P-003 | 正式运行主机唯一为 Linux AMD64；原生命令再次核对 `uname` | `deploy/common.sh`、`scripts/build.sh`、`deploy/xcosctl` | 保障 | 中 | 错误架构可能走到状态创建或 companion 启动后才失败 | Linux x86_64 正例；aarch64/非 Linux 负例 |
| SEN-P-004 | MediaMTX companion 固定为 `v1.20.0 linux_amd64` 和精确 SHA-256 | `config/mediamtx.lock`、`scripts/build.sh`、`deploy/xcosctl start` | 保障 | 高 | API、配置和媒体行为不可复现，发行身份失去意义 | version 输出、platform、binary SHA 三者同时匹配 |
| SEN-P-005 | 控制面和媒体面分离；Rust 不代理 RTSP 输入，也不转码视频 | `src/mediamtx.rs`、`deploy/Caddyfile` | 核心 | 高 | 把媒体搬入 Rust 会重写容量、协议和攻击面；删 companion 则无直播/录像 | Rust 路由不存在 RTSP 转发；MediaMTX path 实测 |
| SEN-P-006 | 普通运行只接受当前合同并验证当前状态身份 | `src/main.rs` CLI、`src/sqlite/` | 保障 | 高 | 加入代际 reader 会长期扩大状态和测试矩阵 | init/run/config validate/status/help/version核心命令和只读state-contract；普通运行拒绝非当前库，不提供历史转换入口 |
| SEN-P-007 | Server 端 React 19 + TypeScript strict + Vite 8 控制台位于 `web/` | `web/package.json`、`web/src/main.tsx` | 建议保留 | 高 | API 和媒体能力仍在，但没有内置可操作控制台 | typecheck、Vite build、发行静态树验证 |
| SEN-P-008 | Server Rust与单个 @xcss/web 包候选以xcss 1.0.0完整revision、URL及真实tarball integrity受控；公共库已正式发布，产品本轮验收和发行状态单独记录 | Cargo、一个 `@xcss/web` 依赖、manifest/lock | 保障 | 高 | 平台行为分叉；独立构建通过不代表主分支改动已纳入产品 Release | [本项目 CI](https://github.com/isarmg/xcos/actions)及[正式发行资产](https://github.com/isarmg/xcos/releases)；后续更新仍须复验锁图和发行身份 |
| SEN-P-009 | `config/` 只存可提交样例和受审 companion 合同；真实 Secret 不进仓库 | `config/xcos.env.example`、`.gitignore` | 开发运维 | 低 | Secret 容易误提交，或部署字段缺少审查入口 | Secret 扫描；样例字段与 parser 对照 |
| SEN-P-010 | 原生发行树提供单一xcosctl生命周期入口，仓库另给完整systemd部署示例；两者不能同时管理相同进程和数据 | `deploy/xcosctl`、`deploy/.*-action.sh`、`deploy/*.service` | 开发运维 | 中 | 双重进程管理可能启动半套服务或竞争端口 | xcosctl临时根测试通过；systemd示例尚未在真实主机执行 |

## 3. 配置、启动和生命周期

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-C-001 | 生产环境文件固定为 `/etc/isarmg/xcos.env`，而不是项目子目录 | `deploy/common.sh`、`config/*.env.example` | 开发运维 | 中 | 多路径来源会造成操作者修改无效文件或混合配置 | bootstrap、start、文档和测试使用同一平面路径 |
| SEN-C-002 | `xcosctl bootstrap` 排他创建配置、状态和运行目录，不覆盖既有环境文件 | `deploy/xcosctl`、`deploy/.bootstrap-action.sh` | 保障 | 高 | 重跑初始化可能无提示覆盖 Secret 或状态定位 | 首次创建、第二次 no-clobber、symlink 负例 |
| SEN-C-003 | bootstrap 写入默认 canonical `BOOTSTRAP_ADMIN_USERNAME=admin`，随机生成 JWT Secret、32 字节 credential key 和初始密码且不回显 | `deploy/.bootstrap-action.sh` | 保障 | 中 | 终端、CI 日志或 shell history 可能泄漏高价值 Secret，或首管身份与 Server parser 漂移 | lifecycle 日志中搜索 Secret；username 精确；文件 mode 0600 |
| SEN-C-004 | 人工确认标记阻止未审阅初始 Secret 直接启动 | `xcos.REVIEW-SECRETS-BEFORE-START`、`xcosctl bootstrap` | 保障 | 低 | 默认凭据可能直接进入运行环境 | 未确认 start 失败；`--confirm-config` 后标记消失 |
| SEN-C-005 | `BIND_ADDR` 缺省值和正式模板均为 `127.0.0.1:8080`；`APP_ENV=development` 进一步禁止显式外部绑定；生产使用 Secure `__Host-` Cookie | `src/config.rs`、`config/xcos.env.example`、`deploy/.bootstrap-action.sh`、`src/auth.rs` | 保障 | 中 | 默认监听任意网卡会绕开 TLS 网关；非 Secure 开发 Cookie 可能暴露到局域网 | 缺省 loopback、IPv4/IPv6 loopback正例；development 外部地址负例；正式样例一致 |
| SEN-C-006 | `APP_JWT_SECRET` 至少 32 bytes，`CREDENTIALS_KEY` 必须标准 Base64 且解码恰为 32 bytes | `src/config.rs` | 保障 | 中 | 弱密钥或歧义 key 长度会降低媒体授权和凭据保护 | 缺失、短值、非法 Base64、31/33 bytes 负例 |
| SEN-C-007 | `XCOS_RUNTIME_DIR` 与开发 `XCSS_DEV_WEB_DIR` 必须绝对路径 | `src/config.rs` | 保障 | 低 | cwd 变化会把锁或前端指到不同位置 | 相对路径拒绝；绝对路径接受 |
| SEN-C-008 | 登录 body、bucket 容量、来源/账户窗口、Argon2 并发与超时采用 xcss 固定认证策略 | `xcss::admin_core::AdministratorPolicyV1` | 保障 | 中 | 私有策略或失效环境变量会让运维误判保护边界 | 共享策略与产品实际登录入口；超时后许可回收；无产品参数覆盖 |
| SEN-C-009 | Media token TTL、状态刷新、reconcile 周期、上游请求超时均显式配置 | `src/config.rs` | 建议保留 | 中 | 删除可调性会把不同网络/规模强行绑定同一节奏 | 0/极端值行为；周期任务不重叠失控 |
| SEN-C-010 | Server 不接收摄像机网络地址或设备凭据；发现、地址校验和适配器配置全部位于 Client | xcoc `device`/`onvif` 模块、Server 当前 Schema | 保障 | 中 | Server 重新接触摄像机内网会绕过授权实例边界 | Server direct API 为 405、Schema 拒绝未绑定摄像机 |
| SEN-C-011 | 正式 `run --release-root` 只使用内嵌 Web，并验证 `share/web-assets.json` 与 binary 清单精确相等 | `src/main.rs` | 保障 | 中 | 可把已验证 Rust 与任意前端混搭 | 内嵌资源完整 HTTP 验收；目录覆盖/重写清单负例 |
| SEN-C-012 | start 按 companion→readiness→应用顺序启动；任一步失败会清理本次启动的进程 | `deploy/xcosctl start`、`deploy/.start-action.sh` | 保障 | 高 | 失败可能遗留孤儿 MediaMTX 或错误 PID 文件 | companion 失败、应用失败、并发 start、回滚 |
| SEN-C-013 | stop 先停应用，再停 MediaMTX，遵循数据库/协调器先释放的顺序 | `deploy/xcosctl stop`、`deploy/.stop-action.sh` | 保障 | 中 | 先停媒体面会扩大 operation 结果不确定窗口 | 正常停止、重复停止、PID 身份不匹配 |
| SEN-C-014 | PID 文件和进程可执行路径共同校验，不只信任 PID 数字 | `deploy/common.sh` | 保障 | 中 | PID 重用可能误杀无关进程或误报运行状态 | 伪 PID、已退出 PID、不同 executable 负例 |

## 4. Administrator 认证与请求安全

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-A-001 | 所有控制面账户都是 Administrator；平台表不持久化角色；管理员 ID 为不透明 TEXT，不要求 UUID | xcss admin-core/admin-sqlite、生成 Schema | 核心 | 高 | 失去认证或业务外键错误绑定 | 当前 DDL、真实 bootstrap ID、业务外键回归 |
| SEN-A-002 | 三个认证路径精确为 `/api/v1/auth/login`、`/api/v1/auth/session`、`/api/v1/auth/logout` | xcss constants、`src/routes/` | 保障 | 中 | 路径漂移会让共享客户端失效，别名会形成额外攻击面 | 三路径方法矩阵；其他形状 404/405 |
| SEN-A-003 | 登录 body 精确为 `{username,password}`，未知字段与已删除的 email 字段由 DTO 拒绝；Session 精确为 `{authenticated,user_id,username,role:"admin",csrf_token}` | `AdministratorLoginRequest`、`AdministratorSession`、Axum `Json` | 保障 | 低 | 歧义输入可能在 Server、xcss 和 Web 产生不同解释 | 缺失、额外、类型错误、超限 body、Session exact keys |
| SEN-A-004 | username 使用 xcss 唯一规则：candidate 1..64 bytes printable ASCII，经 trim ASCII/lowercase 后 canonical 必须为 3..64 bytes、首尾字母数字、字符仅 `[a-z0-9._-]`；Schema 保存同一 canonical 形状 | `normalize_administrator_username`、`require_canonical_administrator_username`、`current_schema.sql` | 保障 | 中 | 同一管理员可用变体绕过唯一约束/限流，或跨产品身份语义不一致 | 大写/首尾空白正例；`@`、Unicode、内部空白、控制字符、首尾分隔符负例 |
| SEN-A-005 | 密码只接受 xcss 当前策略和精确 Argon2id hash 参数 | `xcss::admin_auth`、`src/auth.rs` | 保障 | 高 | 放宽 hash 会形成多策略验证分支；弱 hash 降低离线攻击成本 | 当前 PHC 正例；参数、版本、salt/output 偏差拒绝 |
| SEN-A-006 | 未知账户使用当前 dummy hash，减少账户枚举时序差异 | xcss AdministratorService | 保障 | 中 | 未知 username 明显更快返回 | 已知错误密码与未知账户成本 |
| SEN-A-007 | 登录按来源 IP 与 canonical username 分别限流，全局 bucket 有界 | xcss AdministratorService | 保障 | 高 | 暴力猜测或耗尽认证资源 | 规范化、窗口恢复、有界容量、429 |
| SEN-A-008 | Argon2 计算使用共享 semaphore 和等待预算 | xcss AdministratorService | 保障 | 高 | blocking worker 耗尽 | 许可上限、超时、失败释放 |
| SEN-A-009 | Session token 为 32 随机字节，平台库仅保存 SHA-256 digest | xcss _common_admin_sessions | 保障 | 高 | 明文库可转为活跃登录凭据 | token/digest 形状、无明文 |
| SEN-A-010 | Session 具有固定 idle/absolute TTL，平台节流刷新 last_seen | xcss authenticate_session | 保障 | 高 | 会话永久存活或写入过密 | 过期、刷新预算、CSRF 比较更新、时间不倒退 |
| SEN-A-011 | 改密/停用增加 session_version，并原子撤销该账户全部 Session | xcss manage_administrator | 保障 | 高 | 旧会话继续控制设备 | 改密、停用、审计回滚、失效 Cookie |
| SEN-A-012 | 生产 Cookie 为 __Host-admin-xcos-session，Secure/HttpOnly/SameSite=Strict/Path=/ | xcss admin-core/admin-axum | 保障 | 低 | 窃取与跨站风险扩大 | Set-Cookie 精确属性、开发 Cookie |
| SEN-A-013 | logout 撤销 Session、提交平台安全审计并过期 Cookie | xcss AdministratorService/admin-axum | 建议保留 | 低 | 无法主动结束会话 | 注销后 401、Cookie 清理 |
| SEN-A-014 | 恢复 Session 以 CAS 轮换 CSRF 摘要，迟到的 restore/touch 不能恢复旧摘要 | xcss rotate_session_csrf | 保障 | 高 | CSRF 轮换可被并发请求撤销 | SQLite/Static CAS、旧/新摘要、错误映射 |
| SEN-A-015 | unsafe 请求要求单个 `X-CSRF-Token` 且 constant-time 比较 digest | `enforce_browser_security`、xcss helper | 保障 | 高 | 已登录浏览器可能被跨站触发控制动作 | 缺失、重复、逗号合并、错误、正确 token |
| SEN-A-016 | 浏览器请求要求严格同源 Origin/Host/URI authority 与 `Sec-Fetch-Site: same-origin` | `require_administrator_same_origin`、`src/auth.rs` | 保障 | 高 | 代理歧义或跨站请求可能绕过 CSRF 边界 | HTTP/1 Host、HTTP/2 authority、重复头、cross-site |
| SEN-A-017 | 认证、业务和路由 rejection 使用 xcss `ErrorEnvelope` | `src/error.rs`、`xcss::error` | 保障 | 中 | Web 无法稳定按 code/retryable 处理，内部错误可能泄漏 | 400/401/403/404/409/429/500 exact envelope |
| SEN-A-018 | 管理 Web 通过公共当前账号接口修改自己的 username 和密码，不挂载管理员创建/列表/停用路由 | `xcss::admin_axum` 当前账号路由、Shell 账号设置 | 保障 | 高 | 账号入口或凭据变更绕过同源/CSRF 和原子会话撤销 | 原密码核验、改名/改密、旧会话撤销、过期 Session 与 CSRF 拒绝 |
| SEN-A-019 | 管理员写入与安全审计同事务；密码/停用包含会话撤销审计；成功登录与 Session 创建审计也原子提交 | xcss admin-sqlite | 保障 | 高 | 状态与审计分叉 | 审计故障回滚、actor/subject/request ID，无凭据泄漏 |

## 5. 摄像机实例与客户端边界

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-K-001 | Server 的一个授权实例严格对应一台 Client 上报摄像机；同一物理 Client 安装可持有多个授权实例 | `xcocs`、`cameras UNIQUE(client_id)`、edge v1 | 核心 | 高 | 授权、摄像机和录像归属重新变得含糊 | 同 installation 多授权正例；同授权第二台摄像机拒绝 |
| SEN-K-002 | Server 只读列出 Client 上报摄像机；`POST/PUT/DELETE /cameras` 和 Server ONVIF discovery 均不存在 | `routes::router`、当前 Schema | 保障 | 高 | 可绕过 Client 与永久授权码直接创建摄像机 | method/path 负例、Schema direct insert 负例 |
| SEN-K-003 | 多品牌差异由 Client 的适配器归一为统一 identity、capabilities、streams 与状态快照 | xcoc `device`/`onvif`、`ClientCameraSnapshot` | 核心 | 高 | Server 会重新耦合厂商协议 | RTSP/ONVIF 输出同一严格 DTO，拒绝厂商私有字段 |
| SEN-K-004 | 永久授权码使用认证加密保存并绑定授权实例 ID；Server 可查看和更换，更换立即撤销旧 Token | `SecretBox`、`authorization_code_enc`、`update_client_authorization` | 保障 | 高 | 数据库泄漏可直接暴露配对权限，或旧客户端继续连接 | 错实例/错 key/篡改失败，更换后旧凭据失败 |
| SEN-K-005 | 摄像机 RTSP/ONVIF 地址和设备账号只保留在 Client；Server Schema 不存在这些列 | `cameras`、Client LocalState | 保障 | 高 | Server 数据库会暴露摄像机内网和设备 Secret | Schema 列清单、管理 JSON 均无 URL/密码 |
| SEN-K-006 | Client 快照必须恰好一台摄像机且摄像机 ID 等于授权实例 ID | `client_snapshot`、edge v1 | 保障 | 高 | 一个授权码可越权覆盖其他摄像机 | 0/2 台、错 ID、重复 ID 均拒绝 |
| SEN-K-007 | 启动与 doctor 认证全部持久授权码 envelope | `doctor::verify_credentials`、`SecretBox` | 保障 | 高 | 错 key 或坏密文直到配对管理时才暴露 | 任一授权码篡改使检查失败且不改库 |
| SEN-K-008 | 撤销实例会停用并隐藏摄像机、排队清理 MediaMTX；清理确认后可永久删除摄像机和授权实例 | `revoke_client`、media reconciliation | 保障 | 高 | 外键使已撤销实例永久无法删除，或媒体路径残留 | 未清理时冲突；成功清理后二次删除成功 |
| SEN-K-009 | PTZ 只接受 move/stop，pan/tilt/zoom 各在 `[-1,1]`，并作为短时命令发送到拥有该摄像机的 Client | `PtzRequest`、`device_commands`、`routes::ptz` | 可选 | 中 | 删除后仍可监看但不能从控制台云台控制 | 边界值、离线、无能力、命令到期 |

## 6. 期望态、持久操作与协调器

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-O-001 | Client 快照新增/变更摄像机，以及授权更换/撤销，在同一 SQLite 事务写 desired state 与 operation | `client_snapshot`、`queue_camera_change` | 核心 | 高 | HTTP 成功后没有可恢复的 MediaMTX 意图 | 事务失败零部分写；operation 持久化 |
| SEN-O-002 | 每摄像头 desired `generation` 单调增长，operation 绑定 generation | `media_desired_states`、`_common_operations` | 保障 | 高 | 陈旧 worker 可能覆盖更新后的期望状态 | 连续变更、旧 generation 完成、删除后更新 |
| SEN-O-003 | operation 状态包含 pending/running/succeeded/failed/unknown/dead_letter/resolved | `schema/generated/current_schema.sql`、`MediaOperationView` | 核心 | 高 | 无法区分可重试失败、成功和外部效果不确定 | 合法转换、非法组合、终态字段 |
| SEN-O-004 | 公共 operation 的去重身份绑定摄像头与 generation；活跃 target 唯一索引避免同一 namespace/target 同时存在 running/unknown 操作 | `_common_operations`、`_common_operations_active_target`、`enqueue_operation_in` | 保障 | 高 | 同一期望或同一目标可能被重复执行 | generation 去重、并发领取、同 target 活跃冲突 |
| SEN-O-005 | 全局 singleton lease 保证同一时刻只有一个 reconciler owner | `media_reconciler_leases`、`acquire_reconciler_lease` | 保障 | 高 | 多 worker 可同时操作 companion | 双 claim、过期接管、健康 owner 不被抢占 |
| SEN-O-006 | 每条 running operation 也有 lease owner/expiry，并在远端调用前后续租 | `claim_next_operation`、`renew_claimed_leases` | 保障 | 高 | 失去 ownership 的 worker 仍可能 finalize | 过期、慢调用、owner mismatch、fencing |
| SEN-O-007 | 启动取得独占实例锁后，把上一进程遗留的全部 running operation 标为 unknown，包括未过期操作租约；全局 lease 保留，运行期仅回收过期操作 | `recover_interrupted_operations` | 保障 | 高 | 混淆重启恢复与运行期回收会误判中断结果或破坏健康所有权 | 重启未过期/过期两组 fixture；全局 lease 保留与 finalize fencing |
| SEN-O-008 | 对上游明确 HTTP 失败与无法证明响应分别分类 failed/unknown | `AppError::Upstream`、`UpstreamUnknown`、`sanitized_failure` | 保障 | 高 | 网络断线会被误报“失败”并诱发重复副作用 | timeout、连接断、明确 4xx/5xx、解析失败 |
| SEN-O-009 | retry 有 attempt、max_attempts、`retry_at` 和有界退避；不可安全重试进入终态 | `retry_delay`、`finish_failure` | 保障 | 高 | 远端故障会热循环或永久不再收敛 | attempt 边界、时间推进、dead_letter |
| SEN-O-010 | superseded operation 明确收口，不执行已被新 generation 取代的意图 | `finish_superseded` | 保障 | 高 | 快速连改会下发过时配置 | 连续更新/删除、队列次序、审计状态 |
| SEN-O-011 | reconciler 按 desired state 新增/更新/删除 main 与可选 sub path | `apply_desired`、`MediaMtxClient::upsert_path/delete_path` | 核心 | 高 | 数据库配置不再作用于真实媒体服务 | 主/子流、enable、record flag、delete |
| SEN-O-012 | MediaMTX source 固定为 Client `publisher`，只持久 source digest；Server 不持有带凭据 RTSP URL | `source_digest`、`media_actual_paths` | 保障 | 中 | 漂移检测可能失真或把设备 Secret 带入 Server | publisher digest、日志脱敏 |
| SEN-O-013 | 周期比较 expected 与 MediaMTX path config/actual publisher/recording 并排队 drift operation | `observe_and_schedule_drift` | 建议保留 | 高 | 外部手改或 companion 重启后持续漂移 | 缺 path、错误 source/record、已一致不重复排队 |
| SEN-O-014 | operation 查询/人工核对 API 返回持久状态；当前实例 Web 不单独暴露任务页 | `/media/operations/{id}`、`resolve_media_operation` | 建议保留 | 中 | 无法诊断 unknown/dead-letter 的收敛结果 | pending→终态、unknown/resolved、404 |
| SEN-O-015 | 当前没有通用请求 Idempotency-Key 或客户端 revision CAS；幂等来自 generation/唯一索引和 desired-state 收敛 | `queue_camera_change`、Schema 索引 | 保障 | 高 | 误以为有 header 级幂等会导致调用方不安全重放 | 文档/API 不声明不存在的 header；并发测试按实际 generation 语义 |
| SEN-O-016 | 媒体状态与观测时效分离：失败/重启保留已知状态，暴露 fresh/stale/unknown、最后成功时间及到期时间 | `background`、`CameraView`、`camera-status.ts` | 保障 | 高 | 清单失败或刷新停止后可能无限显示历史在线；误置离线会歪曲设备事实 | 本地合成清单 503/无效响应、空清单、恢复、过期与启动失效 |

## 7. MediaMTX、直播、录像和媒体授权

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-M-001 | stream ticket 只为已启用摄像头的 main/sub profile 签发 | `/cameras/{id}/stream-ticket` | 核心 | 高 | 浏览器无法获得受限媒体入口；放宽则可读不存在/停用资源 | main/sub、无子流、disabled、未知 camera |
| SEN-M-002 | 媒体 JWT 使用 HS256 key，经 HKDF 从 `APP_JWT_SECRET` 派生 | `issue_media_token`、`media_signing_key` | 保障 | 高 | 直接复用根 Secret 或弱 key 会扩大泄漏影响 | key 派生确定性、错 Secret、算法固定 |
| SEN-M-003 | 浏览器读取与 Client 发布都使用短时 JWT；严格绑定 protocol、issuer、audience、kind、subject、camera、path、actions、jti、iat/nbf/exp，长期 Client API Token 不进入媒体 URL | `client_publish_url`、`MediaClaims`、`decode_media_token` | 保障 | 高 | Token 可跨产品、跨摄像头、跨用途重放，或媒体链路泄漏扩大到控制面 | 每字段篡改、未知字段、read/publish 隔离、时间窗、jti |
| SEN-M-004 | MediaMTX HTTP auth callback 有 4 KiB/字段上限并核对 path 与 action | `/internal/v1/media/auth`、`MediaAuthRequest` | 保障 | 高 | callback 可被超大字段耗尽，或 Token 越权到其他 path | 超限、错 path/action、过期、额外字段 |
| SEN-M-005 | WHEP 浏览器播放器生成 recvonly offer、等待 ICE、设置 answer 并 DELETE resource | `web/src/whep.ts` | 核心 | 高 | 失去低延迟直播或遗留服务端 WHEP Session | 成功连接、12 秒 timeout、close、unmount |
| SEN-M-006 | WHEP OPTIONS/POST 与资源 DELETE 携带短时 Bearer；请求前验证 ticket 和资源 Location 与应用同源且无 userinfo | `WhepPlayer`、`requireSameOriginMediaUrl`、`whepResourceUrl` | 保障 | 高 | 删除校验可能把 Token 发往意外 origin | 同源相对/绝对地址正例、跨源及 userinfo 拒绝；生产代理路径匹配 |
| SEN-M-007 | 录像列表通过 MediaMTX playback API，查询可选 start/end 和指定 camera/profile | `list_recordings`、`MediaMtxClient::recordings` | 建议保留 | 高 | 直播保留，但无法定位历史片段 | 时间范围、main/sub、无录像、上游错误 |
| SEN-M-008 | 录像播放只允许 mp4/fmp4，单次 0.1 秒至 6 小时 | `play_recording` | 保障 | 中 | 无边界请求可放大上游和带宽资源消耗 | duration 边界、format、非法时间 |
| SEN-M-009 | 播放代理只转发 Content-Type/Length/Range/Disposition 白名单响应头并流式正文 | `play_recording` | 保障 | 高 | 全量透传上游头可能改变安全策略；整段缓冲会耗内存 | 200/206、Range、上游错误、大正文 |
| SEN-M-010 | MediaMTX 录制 fMP4、15 分钟 segment、默认保留 168 小时 | `config/mediamtx.yml` | 建议保留 | 中 | 删除 record 失去历史回放；改保留期直接改变容量需求 | config lock、record path、过期清理实测 |
| SEN-M-011 | start 通过环境把录像根固定到 `/var/lib/isarmg/xcos/recordings` | `MTX_PATHDEFAULTS_RECORDPATH`、`deploy/.start-action.sh` | 保障 | 中 | inert 样例路径或 cwd 可能成为真实写入位置 | 进程环境、路径权限、release relocation |
| SEN-M-012 | Caddy 将 `/media-webrtc/*`、`/media-hls/*` 与应用汇聚到一个浏览器 origin；三个上游精确为本机 `127.0.0.1:8889/8888/8080`，不支持容器 DNS 别名 | `deploy/Caddyfile`、CI proxy gate | 保障 | 中 | 跨 origin 会复杂化 Cookie、CORS 和媒体授权；容器名在当前原生部署中无法解析 | 根级副本缺失；WHEP/HLS/API 同源；管理端口不公网暴露；拒绝 `app:`/`mediamtx:`；生产设置真实 `SITE_ADDRESS` |
| SEN-M-013 | MediaMTX API、metrics、playback 固定 loopback；生产启动器强制受信证书的 RTSPS 8322，HLS 8888、WebRTC HTTP 8889 与 UDP 8189 绑定主机网卡。防火墙分别限制 Client 发布、Caddy 上游和浏览器 UDP | `src/config.rs`、`deploy/xcosctl`、内部 bootstrap/start 动作 | 保障 | 高 | 明文发布可能暴露媒体 Token；错把媒体 listener 当成 loopback 会令远程 Client 无法发布 | rtsps-only、证书/私钥检查、loopback 拒绝、listener 与 NAT 验收 |

## 8. 事件、状态与审计

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-E-001 | 后台周期读取 MediaMTX path，将摄像头状态更新为 online/offline/disabled | `background::refresh_statuses` | 核心 | 高 | UI 状态长期停留 pending，无法判断设备可用性 | publisher 出现/消失、disabled、last_seen |
| SEN-E-002 | 状态变化写 `events`，severity 只允许 info/warning/critical | `background::emit_event`、Schema CHECK | 建议保留 | 中 | 设备上下线没有持久事件记录 | online/offline、pending 抑制、非法 severity |
| SEN-E-003 | 事件列表支持 camera 和 1–100 条 limit | `EventQuery`、`list_events` | 建议保留 | 中 | 大量事件无法按当前需求过滤，或响应无界 | filters、limit clamp、排序 |
| SEN-E-004 | 事件确认记录时间和 Administrator ID | `ack_event`、`acknowledged_*` | 建议保留 | 中 | 告警无法形成最小人工闭环 | 不存在 ID、重复确认、CSRF、账号删除后的 FK |
| SEN-E-005 | SSE 只作实时通知，SQLite 是事实源；lagged 时发送 `resync-required` 后断开 | `event_stream`、broadcast channel | 保障 | 高 | 静默跳过会让页面误以为事件完整；删 SSE 则只能轮询 | 正常事件、lag、关闭、重新全量查询 |
| SEN-E-006 | 审计表记录用户、动作、实体、细节和时间；查询最多 500 条 | `audit_logs`、`list_audit` | 建议保留 | 中 | 敏感变更追溯能力下降 | client create/pair/rotate/revoke/delete、PTZ queue、login；limit clamp |
| SEN-E-007 | 实例持久变更审计与业务同事务，PTZ 排队的产品审计为 best-effort；管理员登录审计由 xcss 同事务提交 | `write_audit_in`、`write_audit`、xcss admin-sqlite | 保障 | 高 | 若把两类语义混同，运维会错误承诺审计不丢 | DB 故障注入；产品与平台审计语义分别说明 |
| SEN-E-008 | 公共operations audit outbox与后台投递任务以事件ID幂等物化审计 | xcss xcss::operations、operation-audit监督任务 | 核心 | 高 | 审计提交/确认同事务，投递失败保留积压并表达降级，不重放业务动作 | 文档、Schema 和代码搜索一致 |
| SEN-E-009 | system status 汇总数据库、MediaMTX 与已配置服务器录像数；实例总数和在线数由实例列表统一计算 | `/system/status` | 建议保留 | 低 | 控制台缺少一页式运行概况 | companion 不可达、坏 credential、空设备 |

## 9. React/Vite 管理 Web

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-W-001 | 共享 Shell 负责登录、恢复、退出、导航、主题、诊断、通知和安全错误；产品只传身份和业务页面 | createXcssAdminApplication | 保障 | 高 | 产品复制平台状态机 | xcss 10 项浏览器验收及消费者浏览器回归 |
| SEN-W-002 | 页面只在内存持有 Session/CSRF；Cookie 由浏览器 HttpOnly 管理 | `@xcss/web/admin-web`、`@xcss/web/http-client` | 保障 | 高 | 把 Secret 放 local/sessionStorage 会扩大 XSS 泄漏 | storage 扫描、刷新、401 清理 |
| SEN-W-003 | 所有业务响应经过 TypeScript runtime guard 检查必需字段/类型，不只依赖静态类型 | `web/src/api.ts` | 保障 | 高 | 异常或漂移 JSON 会在组件深处被错误使用 | 缺失/错误类型、数组成员；产品 guard 当前容忍额外响应字段 |
| SEN-W-004 | 授权实例列表按名称一次返回全部记录；详情展示 Client 上报摄像机、搜索和卡片直播，并由实例设置管理媒体期望 | `ClientDetails`、`CameraView`、`ClientSettings`、`list_clients` | 核心 | 高 | 失去实例管理旅程或无法核对 Client 上报设备 | 空态、搜索、完整有序实例列表、实例设置与媒体操作；Server 不直接添加/编辑摄像头凭据 |
| SEN-W-005 | 详情使用共享 Dialog，主码流及鼠标/键盘 PTZ；move/stop 串行，松开、取消、失焦及关闭均触发停止 | CameraDrawer | 可选 | 中 | 缺少精细控制或停止竞态 | pointer cancel、Space/Enter、窗口 blur、关闭清理 |
| SEN-W-006 | Recordings 页面按摄像头和时间范围查询并播放 | `RecordingsView` | 建议保留 | 中 | API 尚在但普通用户难以回放 | 无摄像头、无结果、播放 URL 清理 |
| SEN-W-007 | Events 页面筛选未确认、手动刷新和确认事件 | `EventsView`、SSE effect | 建议保留 | 中 | 事件 API 无内置操作界面 | SSE resync、确认、camera name 映射 |
| SEN-W-008 | 系统页组合媒体状态与业务审计；管理员账号仅由 xcss Shell 右上角人物图标设置；不请求 /users | SystemView、xcss AccountSettings | 建议保留 | 中 | 业务状态缺失或账号入口分散 | 系统状态、业务审计、账号设置、无管理员列表 |
| SEN-W-009 | xcss design tokens、scoped reset、focus/reduced-motion/forced-colors 基线 | CSS imports、`data-xcss-scope` | 保障 | 中 | 基础交互和可访问性在项目间漂移 | CSS 摘要、键盘焦点、减弱动态、高对比度 |
| SEN-W-010 | 产品 CSS 仅维护业务布局，颜色/字体/控件来自 xcss；视频黑底属于媒体业务 | web/src/styles.css | 建议保留 | 中 | 私有平台样式导致主题和可访问性漂移 | 无 token 覆盖、无私有字体、移动明暗主题 WCAG AA |
| SEN-W-011 | WHEP player 在 component cleanup、profile/camera 变化时关闭 peer/resource | `LiveVideo` effect、`WhepPlayer.close` | 保障 | 高 | 切页后仍保留媒体连接和资源 | mount/unmount、快速切换、失败重试 |
| SEN-W-012 | 精确 Node 26.7.0、React/DOM 19.3.0、TS 7.0.2、Vite 8.3.3 工具链 | `.node-version`、`package.json`、lockfile | 开发运维 | 中 | CI/开发/发行 bundle 不可复现 | clean `npm ci`、engine、lock 来源、typecheck |
| SEN-W-013 | `build` 强制先执行 `check:xcss`，再 strict typecheck 与 Vite build | `package.json`、`web/tests/design-xcss.test.mjs` | 开发运维 | 中 | 共享依赖或 CSS 漂移时仍可能生成表面可用 bundle | 故意改版本/import/scope 后 build 在 bundling 前失败 |
| SEN-W-014 | 发行内嵌全部 Web；只附带 `share/web-assets.json`，不需要运行时 npm/CDN | Vite output、`scripts/build.sh`、release manifest | 保障 | 高 | 运行时网络依赖会破坏离线部署和制品身份 | 断网加载、资源引用、额外文件/篡改拒绝 |

## 10. SQLite、锁、doctor、发行与供应链

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-R-001 | 主文件不存在时只创建当前 Schema；已有空文件或非当前库拒绝 | `sqlite::prepare_current_database` | 保障 | 高 | 自动补表会把未知状态变成不可审计混合状态 | 不存在、空文件、错 application/version/revision/SHA |
| SEN-R-002 | xcss `xcss::schema_identity` 统一验证 `product_metadata` DDL/完整列形状、exact identity 与现场 `sqlite_schema` fingerprint；Xcos 使用 xcss 的 SQLx 当前 Schema adapter 和自身 lease 校验 | `validate_current_connection`、`product_metadata_rows`、`schema_rows` | 保障 | 高 | 手改 metadata 可伪装结构，DDL drift 被忽略，或多产品 fingerprint 算法分叉 | metadata 0/2 行、负 revision、错误 storage class/default/列、额外表/索引、只改 SHA、WAL 只读拒绝 |
| SEN-R-003 | 验证前从 main/WAL/journal 复制私有 generation，并复核源文件身份未变 | `snapshot_generation` | 保障 | 高 | 验证可能读到跨时刻混合字节，或在源库上产生写入 | WAL、并发变化、symlink、generation cleanup |
| SEN-R-004 | SQLite integrity、foreign key 和 rollback write probe 用于 doctor | `doctor.rs`、`sqlite.rs` | 开发运维 | 高 | 仅靠 `SELECT 1` 无法发现损坏、FK 或不可写 | corruption、FK、read-only、写探针回滚 |
| SEN-R-005 | global lease singleton 除 DDL 外还验证 owner UUIDv4、RFC3339 时间和字段组合 | `validate_global_lease_values` | 保障 | 高 | 非法业务不变量可通过 Schema SHA 后进入 worker | 多行、缺行、非规范 UUID/时间、expiry 顺序 |
| SEN-R-006 | Application lock 绑定数据库身份、runtime 目录和 release 路径 | `src/runtime_lock.rs` | 保障 | 高 | 双实例可并发 claim、改库和控制 companion | 同库同/不同 runtime；symlink/hardlink；PID |
| SEN-R-007 | maintenance lock 为外部停机工具保留排他协调边界 | `DatabaseMaintenanceLock` | 保障 | 高 | 备份/恢复工具可能与运行服务同时操作组合状态 | 服务共享持有、维护排他、锁顺序 |
| SEN-R-008 | doctor 验证 release、数据库、凭据、recordings 根、MediaMTX binary/config/contract | `src/doctor.rs` | 开发运维 | 高 | 上线验收只能依靠零散命令，难以证明组合一致 | offline 正反例；每项单独篡改 |
| SEN-R-009 | online doctor 额外探测应用与 MediaMTX loopback readiness | `DoctorOptions.offline`、`live_probe` | 开发运维 | 低 | 只能证明静态状态，不能证明两个进程正在响应 | offline 不依赖进程；online 一项失败即报告 |
| SEN-R-010 | release identity 绑定产品、版本、source revision、target、API、Schema、Web、credential 与 MediaMTX | `src/release.rs::ReleaseIdentity` | 保障 | 高 | 可把不同提交/协议/companion 拼成同名发行物 | identity JSON 与 manifest header 一致 |
| SEN-R-011 | 全树 manifest 精确验证 path/type/mode/size/SHA，拒绝额外条目 | `verify_release`、xcss `xcss::web_assets` | 保障 | 高 | 攻击者或误部署可插入/替换资产而仍启动 | missing/extra/tamper/mode/symlink/hardlink |
| SEN-R-012 | release root 必须是规范物理版本路径，正式父目录 root-owned | `validate_release_root`、`PRODUCTION_RELEASE_ROOT` | 保障 | 高 | 可通过 alias 或可写父目录替换已验证内容 | symlink parent、相对路径、错误 suffix、ownership |
| SEN-R-013 | `scripts/build.sh` 要求 clean checkout、annotated `v1.0.0` 指向 HEAD 和 Linux AMD64 | `scripts/build.sh` | 开发运维 | 中 | 无法把制品稳定追溯到源码与版本 | dirty tree、lightweight/wrong tag、wrong host |
| SEN-R-014 | build 在同一文件系统 stage，验证后 no-clobber 安装固定发行目录 | `scripts/build.sh` | 保障 | 高 | 半写 release 或同版本覆盖会让重启内容不可预测 | 中途失败、并发 build、第二次 build |
| SEN-R-015 | lifecycle test 使用临时根覆盖 no-clobber、Secret、锁、失败回滚和链接防御 | `scripts/lifecycle-test.sh` | 开发运维 | 高 | 脚本安全语义容易在普通单元测试外回归 | 临时根运行；不得访问真实 `/var/lib` |
| SEN-R-016 | relocated smoke 使用真实 Rust/Vite/SQLite/MediaMTX 制品验证重定位和篡改拒绝 | `scripts/relocated-smoke-test.sh` | 开发运维 | 高 | 静态脚本检查无法证明真实发行闭包 | 真实启动、hashed assets、字节篡改、source-bound binary |
| SEN-R-017 | CI 同时门禁 Rust fmt/check/clippy/test、Web、native 生命周期与 Caddy 当前代理合同 | `.github/workflows/ci.yml` | 开发运维 | 高 | 任一语言或交付层可独立漂移进入 main；代理可能重新指向不存在的容器 | clean checkout 全 job；锁文件模式；根级 Caddyfile/容器上游负例；三个 loopback 上游精确一次 |
| SEN-R-018 | Rust 固定 1.99.0，Cargo.lock 与 npm package-lock 都纳入提交 | `rust-toolchain.toml`、lockfiles | 开发运维 | 中 | 依赖解析随时间变化，构建结果不可复现 | `--locked`、`npm ci`、工具链版本 |
| SEN-R-019 | 源配置统一为 `config/`，主机部署资产为 `deploy/`，客户端为 `web/`，生命周期为 `deploy/`，构建、检查、打包入口为 `scripts/`；根目录不放散落部署文件 | 仓库目录结构、CI proxy gate | 开发运维 | 低 | 配置、客户端和部署资产散落，开发者难以判断事实源；双份代理模板会漂移 | 目录清单；根级 `Caddyfile` 不存在；脚本/文档不引用已移除位置 |
| SEN-R-020 | 当前Schema identity为application `xcos`、数据格式`xcos-db-v2`、revision2、SHA `4d20083821ff39d78792d0795b26206e851c2e6d0523109ee49cfc06666a1d4a`；`_common_administrators` 使用 username，不保存 email/role | `schema/generated/current_schema.sql`、`src/sqlite/`、`scripts/lifecycle-test.sh` | 保障 | 高 | 发行物、运行库和运维文档可能各自接受不同管理身份 DDL | code-owned fingerprint 重算、metadata/现场 schema、列清单、lifecycle identity 一致 |

## 11. 可观测性、容量和故障边界

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| SEN-Q-001 | `/healthz` 只证明进程能响应；`/readyz` 同时要求 DB/凭据与 MediaMTX 健康 | `routes::live/ready` | 开发运维 | 中 | 编排器无法区分存活和可服务 | companion down、坏 credential、DB down |
| SEN-Q-002 | tower HTTP trace 和结构化应用日志提供请求/后台错误线索 | `TraceLayer`、`tracing` | 开发运维 | 中 | 线上请求与 reconcile 故障难关联 | status/latency；禁止记录 Token、密码、完整 RTSP URL |
| SEN-Q-003 | 上游错误只持久化/返回固定脱敏类别，不保存远端正文和 Secret | `sanitized_failure`、`AppError` | 保障 | 高 | 摄像头凭据或内网内容可能进入 DB、JSON、Journal | 故意含 Secret 的上游错误负例 |
| SEN-Q-004 | 事件 broadcast 容量固定 256，lag 通过 resync 协议显式暴露 | `broadcast::channel(256)` | 保障 | 中 | 无界内存或静默丢实时通知 | 超 256 事件、慢消费者、SQLite 回查 |
| SEN-Q-005 | 录像容量和保留由 MediaMTX `recordMaxPartSize`、segment 和 deleteAfter 约束 | `config/mediamtx.yml` | 保障 | 中 | 单文件/总保留失控可能填满磁盘 | 文件增长、168h 清理、磁盘/inode 监控；应用当前不建录像 inventory 表 |
| SEN-Q-006 | HTTP JSON、login、Client 快照、媒体 auth 字段和 playback duration 都有显式上限 | routes/config/models | 保障 | 高 | 外部输入可无界消耗内存、CPU、连接或上游带宽 | 每个上限的边界和恢复测试 |
| SEN-Q-007 | SQLite 是事件与 operation 的持久事实源；SSE、React state 和日志只是投影 | Schema、routes、Web effects | 保障 | 高 | 重启或断线后页面内存会被误当作最终事实 | 重连全量读取、进程重启、SSE lag |

## 12. 明确边界与取舍

| ID | 当前决定 | 实现/边界锚点 | 分类 | 复杂度 | 若改变会发生什么 | 实施前最低证据 |
|---|---|---|---|---|---|---|
| SEN-X-001 | 不提供 observer/operator/viewer 或任何 RBAC 开关 | 无 role 列；所有业务 route 解析 `CurrentUser` | 核心 | 高 | 需重做权限矩阵、Session contract、Web 条件展示、审计与持久结构 | 独立授权设计、逐路由测试、Schema 与 xcss 决策 |
| SEN-X-002 | 不提供内置 TLS 或应用层 HTTPS 强制；由 `deploy/Caddyfile` 所示同源网关终止 HTTPS，后端默认 loopback 并必须由防火墙隔离 | `deploy/Caddyfile`、`src/config.rs`、正式环境样例 | 保障 | 高 | 后端直连会让登录密码/Session 经过明文；内置 TLS 则需承担证书、续期和监听安全 | 生产 `SITE_ADDRESS`、真实证书、三个本机上游、后端不可公网直连；默认 `:80` 仅是模板占位 |
| SEN-X-003 | 不提供视频转码、AI、人脸识别或语义搜索 | 无相关 worker/model/schema | 核心 | 高 | 增加 GPU/CPU、模型供应链、生物特征隐私和派生物状态 | 独立 RFC、资源预算、隐私删除和失败恢复 |
| SEN-X-004 | 不提供云多租户或组织隔离；一个部署是一套 Administrator 与摄像头 | 数据模型无 tenant | 核心 | 高 | 所有查询、JWT、录像路径和审计都要加入租户边界 | 威胁模型、逐查询隔离、计费/配额设计 |
| SEN-X-005 | 不提供运行时 Schema migration、双读或非当前密文 keyring | `src/sqlite/`、`src/crypto.rs` | 保障 | 高 | 产品复杂度会随代数增长，并在启动期写未知数据 | 产品保留单一当前格式；非当前格式明确拒绝 |
| SEN-X-006 | 不提供通用操作 Idempotency-Key；camera desired generation 是当前收敛语义 | Schema/queue 实现 | 保障 | 高 | 新 header 必须定义存储期限、payload digest、冲突和重放响应 | API/Schema/容量/清理/并发完整设计 |
| SEN-X-007 | PTZ 持久化为有期限设备命令，HTTP 202 仅证明已排队；媒体 operation API 不提供设备命令终态查询 | `routes::ptz`、`device_commands`、Client snapshot | 可选 | 高 | 若提供设备副作用终态查询需定义专用查询与不确定结果模型 | 命令到期、Client 去重、回执、重复动作风险 |
| SEN-X-008 | 提供媒体 operation 到本地业务审计的持久 outbox，不提供外部必达 sink | `_common_operation_audit_outbox`、`flush_operation_audit` | 可选 | 高 | 外部导出仍需独立投递、重试、死信、脱敏和容量模型，不能把本地确认当外部送达 | 本地投递/确认同事务；若新增外部 sink 须有独立合同与故障验收 |
| SEN-X-009 | 不承诺应用核对每个录像文件的 Hash/inventory；doctor 目前检查安全目录和写探针 | `doctor::recording_write_probe` | 建议保留 | 高 | 若新增完整 inventory，doctor 时间、存储和备份合同都会扩大 | 百万文件预算、增量索引、特殊文件与并发写设计 |
| SEN-X-010 | 发行树提供 xcosctl；仓库另提供需审阅的 systemd 部署示例 | `deploy/`、release layout | 开发运维 | 中 | systemd 直接跟踪应用和 companion，不能与 xcosctl 后台模式并用 | 发行/运维文档明确；lifecycle 测试 |
| SEN-X-011 | Server 只支持 Linux x86_64 GNU；管理界面为浏览器，边缘客户端平台由独立 Client 仓库定义 | compile/runtime gates、xcoc | 核心 | 高 | 扩平台不能只删除 compile gate，还需 companion、脚本、锁和发行等价证明 | 新平台完整 CI、真实媒体和部署安全验证 |

## 13. 关键取舍说明

### 13.1 为什么只有 Administrator

当前部署目标是单一可信管理域。保留一个角色让每个已认证业务路由的含义明确，避免“按钮隐藏但 API
仍可调用”、事件查看与摄像头密码管理权限错位等问题。`role:"admin"` 留在 wire 中是 xcss 的跨
项目身份常量，不表示数据库存在 RBAC。

### 13.2 媒体期望态与设备命令

摄像头配置是长期期望态，必须在重启后继续收敛，所以使用 durable operation、generation 和 lease。
PTZ 写入带 10 秒有效期的 `device_commands`，HTTP 202 表示排队成功；Client 按命令 ID 去重、执行前
检查截止时间，并通过快照回报结果。排队成功与设备动作成功是两个状态；结果不确定时不能盲目重放 move。

### 13.3 为什么固定 MediaMTX

Xcos 依赖 MediaMTX 的配置字段、API path、WHEP/HLS 和 recording 行为。只固定 Rust 而允许任意
companion，无法复现实际媒体面；因此 binary version、platform、SHA、config 和 release manifest 是一个
不可拆分的发行身份。

### 13.4 当前格式检查

启动路径只证明当前Schema、当前credential envelope和当前external key。Xcos不扫描其他目录、不猜测格式，错误输入明确拒绝。

## 14. 功能删除检查表

变更某项能力时，按其 ID 核对生产者、消费者、状态、配置、测试和发行材料。记录用户影响，再以[开发验证](development.md)及对应专项测试确认结果。

多品牌设备边界固定在 `xcoc`：`rtsp` 与 `onvif` 适配器输出相同的设备身份、能力、码流和
健康模型；Server 不持有 Client 摄像头的地址与密码。Client PTZ 使用有期限的 `device_commands` 信封，执行结果随
下一次快照回报。设备状态与 MediaMTX 管线状态分别持久化，任何一层故障都不会被另一层的心跳覆盖。

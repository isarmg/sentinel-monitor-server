# xcos 运维文档

## 媒体操作人工处理与审计

当前 `POST /api/v1/media/operations/{id}/resolve` 要求管理员 Session 和 CSRF，严格请求体为
`{"resolution":"confirmed_succeeded"}`、`confirmed_failed` 或 `unable_to_confirm` 三种决定之一。
确认成功/失败将 Unknown、Failed 或 DeadLetter 标为 Resolved；无法确认仅将 Unknown 标为 DeadLetter。
这些决定不重放原请求。处理前必须核对 MediaMTX 的实际状态；操作者审计与平台状态同事务提交。

调和结果、摄像头状态、当前 owner 的完成转换与 outbox 同事务。租约过期或本地结果提交失败进入 Unknown，
不作为可重试失败。`operation-audit` 监督任务按事件 ID 幂等物化审计；插入和 outbox 确认同事务。
投递失败会将该 Degrading 任务标为失败并保留积压，诊断可见；排除存储故障后重启恢复投递。

本产品的正式服务端构建和运行平台只有 `x86_64-unknown-linux-gnu`。Linux aarch64、musl、Windows、macOS
以及其他 target 都不属于可部署范围，也没有兼容分支。Rust 工具链固定为 `1.99.0`；Web 构建机固定为
Node `26.7.0`。

## 1. 唯一生产布局

```text
/opt/isarmg/xcos/releases/1.0.0/
├─ RELEASE-MANIFEST
├─ bin/{xcos,mediamtx}
├─ share/web-assets.json
├─ config/{mediamtx.yml,mediamtx.lock}
└─ deploy/{xcosctl,common.sh,.bootstrap-action.sh,.start-action.sh,.status-action.sh,.stop-action.sh}

/etc/isarmg/xcos.env
/var/lib/isarmg/xcos/{db,recordings,logs}
/run/isarmg/xcos/{operations.lock,.state-maintenance.lock,.state-instance.lock,app.pid,mediamtx.lock,mediamtx.pid}

源码 `deploy/Caddyfile` -> 主机受管的 Caddy 配置
```

版本树 root-owned、只读且每个输入为物理路径。核心程序允许唯一受控部署指针 `/opt/isarmg/xcos/current`，该单跳链接仅能指向其所属安装目录 `releases/` 内的当前完整版本树，并继续核对全部 manifest、binary、companion 和静态清单；其他 alias 仍拒绝。原 `xcosctl` 使用版本树物理路径。下面的 systemd 方案使用核心 `current/bin` 入口与独立私有 JSON；程序始终验证指针所指向的完整物理发行树。

## 2. 构建和首次配置

准备 lock 精确匹配的 MediaMTX `linux_amd64 v1.20.0`：

```bash
export XCOS_MEDIAMTX_SOURCE=/absolute/path/to/mediamtx
./scripts/build.sh

/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap
sudoedit /etc/isarmg/xcos.env
/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap --confirm-config
/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl status
```

构建机必须是 Linux x86_64，并安装 Rust `1.99.0` 的 `rustfmt`、`clippy` 组件及
`x86_64-unknown-linux-gnu` target。`scripts/build.sh` 显式使用该 target；`xcosctl start` 会再次拒绝错误运行
平台。不得用修改 manifest 字符串、跳过 target gate 或复制别的平台 binary 形成“临时支持”。

`xcosctl status`展示已核身份的PID状态，并委托当前binary的`status`校验实际readiness和服务身份；停服、未就绪或错误服务保持非零失败，不把HTTP失败静默吞掉。

停止：

```bash
/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl stop
```

Web 和 Rust 由 xcss `xcss-build-server` 按锁图依次构建；配置入口为
`xcss-web-build.json`。产出的实际 executable 必须用 `web-assets` 输出完整清单，与新建 dist
逐项核对大小和 SHA-256。正式发行只携带该清单，不重复携带 HTML/JS/CSS/字体原始树；全部 HTTP 资源
来自同一 executable 内嵌快照。即使重写发行 manifest，清单仍必须逐字节等于执行 binary 的清单。

原生私有环境文件、状态和runtime目录必须属于实际执行用户。配置文件的物理父路径只允许root或该用户拥有且禁止其他用户写；Root不能直接执行服务用户能改写的环境输入。root-owned只读发行树仍可供服务用户执行。

发布器仅接受空的首次发行目的地，存在任何同版本或其他版本发行树都拒绝；不停止现存服务或写入部署指针。第二次 bootstrap 不覆盖既有环境文件。bootstrap 不启动服务，只读取固定的平面
环境路径，也不回显随机 JWT Secret、Credential Key 或临时管理员密码。首次生成的
`PUBLIC_RTSP_PUBLISH_BASE_URL=REPLACE_WITH_PUBLIC_RTSPS_ORIGIN` 是强制审阅占位符；必须改为配对 Client
可达、证书链受 Client 系统信任且名称匹配的 `rtsps://host:8322` origin，并设置
`MEDIAMTX_RTSP_CERT/KEY`，`--confirm-config` 才会接受。生产运行时拒绝明文、loopback/unspecified 发布地址。
`bootstrap --confirm-config`在已审阅配置后，把初始密码通过stdin传入显式`init --username`，创建当前库、管理员和受控日志目录，再执行只读`config validate`。成功才原子移除`BOOTSTRAP_ADMIN_PASSWORD`并清除review标记；失败保留输入和标记。已有库只validate，不重新初始化。普通`start`先validate再执行`run --release-root`，不创建业务数据或管理员。应用结构化轮转日志位于数据库父目录`/var/lib/isarmg/xcos/db/logs`，外层`logs`用于启动器和companion输出。


## 2.1 systemd 与受控 current 部署

仓库 `deploy/xcos.service` 和 `deploy/xcos-mediamtx.service` 是可审阅的 Linux systemd 示例，需要运维创建 `xcos` 账户、安装审核过的完整只读版本树与受控 `current` 指针后使用。此方案与 `xcosctl` 的后台启动方式二选一；不能让二者同时管理同一数据和端口。示例本轮尚未在真实 systemd 主机运行，不把 unit 文件存在称为原生验收通过。

把 `config/xcos.json.example` 配置为 `/etc/isarmg/xcos/config.json`，将私有父目录设为服务用户拥有的 `0700`，文件设为 `0600`；替换全部 secret/公网地址占位符，保留并备份原有 credentials_key。`release_root` 固定为受控 `current`，同一配置来源不能再填写 `mediamtx_config`、`mediamtx_contract`、`mediamtx_binary`；它们由经验证的不可变 bundle 派生。`run --release-root` 明确覆盖配置来源的根选择。应用配置不得混入 companion 的 `MTX_*` 字段；将审阅过的 `config/mediamtx.env.example` 单独存为 `0600` 的 `mediamtx.env`，TLS key 同样只允许服务用户访问。

首次配置完成后，创建私有数据、录像与 runtime 目录，以服务用户交互执行一次核心 `--config ... init --username admin`（管理员密码从 stdin 读取），再 `config validate`。systemd 的 `ExecStartPre` 只校验，`ExecStart` 只运行，不在普通启动自动初始化。已有当前数据必须只读校验，不重置管理员。

```bash
/opt/isarmg/xcos/current/bin/xcos \
  --config /etc/isarmg/xcos/config.json config validate
/opt/isarmg/xcos/current/bin/xcos \
  --config /etc/isarmg/xcos/config.json run \
  --release-root /opt/isarmg/xcos/current
```

安装两个 unit 后以 `xcos.service` 管理部署。主 unit 的 Requires/After 确认 companion 启动依赖；companion 的 PartOf 使停服/重启同时停止其进程，再从 `current` 取得相同已验证版本。所有长驻进程由 systemd 直接跟踪，不能后台化。程序、companion、immutable YAML/lock 和 Web 清单都属于受验证的不可变发行 bundle，不列为可变 `state_paths`；数据库、录像、私有JSON和其他业务状态由只读state-contract与config validate报告。

## 3. 核心配置

实际配置是 0600 文件，`config/xcos.env.example` 只作字段参考。主要字段：

| 类别 | 变量 | 要求 |
|---|---|---|
| 数据 | `DATABASE_URL`、`RECORDINGS_DIR`、`XCOS_RUNTIME_DIR` | 使用固定外部绝对路径 |
| 身份 | `APP_JWT_SECRET` | 至少 32 字符随机值，主机秘密管理 |
| 凭据 | `CREDENTIALS_KEY` | Base64 编码的 32 字节随机值，必须备份到独立秘密系统 |
| 首管 | `BOOTSTRAP_ADMIN_USERNAME/PASSWORD` | 仅全新数据库初始化；默认 username 为 `admin` |
| 环境 | `APP_ENV=production` | 开发模式只允许 loopback |
| 登录 | xcss `AdministratorPolicyV1` 的 body/rate/Argon2 concurrency/timeout | 使用固定公共策略，不提供产品环境变量覆盖 |
| MediaMTX | API、playback、config、contract、binary | 必须指向同一固定 release |
| Web | 内嵌资源与 `share/web-assets.json` | 由实际 binary 精确验证，无生产目录覆盖 |
| 监听 | `BIND_ADDR=127.0.0.1:8080` | 代码默认值和正式样例一致；仅可信本机网关访问 |
| 公网播放 | `PUBLIC_HLS_BASE_URL`、`PUBLIC_WEBRTC_BASE_URL` | 保持同源相对路径，由 Caddy 路由 |
| Client 发布 | `PUBLIC_RTSP_PUBLISH_BASE_URL`、`MEDIAMTX_RTSP_CERT/KEY` | 必填 Client 可达的 RTSPS origin 与受信证书/私钥；生产拒绝明文和 loopback |
| WebRTC 地址 | `MEDIA_PUBLIC_HOSTS` | 填浏览器可达的服务器 DNS/IP，多个值用逗号分隔 |

MediaMTX 的 API `9997`、metrics `9998`、playback `9996` 固定 loopback。生产媒体监听 RTSPS `8322`、HLS
`8888`、WebRTC HTTP `8889` 和 WebRTC UDP `8189` 则绑定主机网卡：8322 是远程 xcoc 上传
视频的入口；8888/8889 通常只由本机 Caddy 代理；8189 是浏览器 WebRTC 媒体候选所需 UDP。防火墙应按
Client 网段、浏览器网络与 NAT 拓扑分别放行，不能把“Caddy 上游使用 127.0.0.1”误解为这些 listener
只绑定 loopback。摄像头本身仍应放入 Client 所在的隔离 VLAN，Server 不需要直达摄像头管理地址。
唯一代理源模板是 `deploy/Caddyfile`。它把 WHEP、HLS 和 Xcos 分别转发到本机
`127.0.0.1:8889`、`127.0.0.1:8888`、`127.0.0.1:8080`，不会解析容器名，也不公开 9996/9997/9998。
模板默认站点占位为 `:80`，不是生产 TLS 证明；生产必须在 Caddy 服务环境设置真实 `SITE_ADDRESS`（或由
配置管理渲染为真实站点），取得有效证书，并用防火墙阻止浏览器直连 Axum/MediaMTX。应用本身不终止
TLS，也不拒绝所有明文直连。若 Client 跨 NAT，必须为 8322 建立受限端口映射，并让
`PUBLIC_RTSP_PUBLISH_BASE_URL` 指向映射后的可达地址；WebRTC 的 8189/UDP 映射和 `MEDIA_PUBLIC_HOSTS`
也必须与浏览器实际访问路径一致。

安装或更新后至少执行：

```bash
# 由配置管理把 deploy/Caddyfile 的唯一站点块纳入主机受管配置；不要盲目覆盖其他站点。
sudoedit /etc/caddy/Caddyfile
sudo env SITE_ADDRESS=xcos.example.org \
  caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sudo systemctl reload caddy
```

若主机 Caddy 使用拆分 include 目录，应由配置管理安装到该目录，不要再保留根级 `Caddyfile` 副本。
仓库没有 Docker/Compose 部署合同，`app:8080` 与 `mediamtx:8888/8889` 都是已删除的未定义目标。

## 4. 当前数据库与凭据合同

`product_metadata` 必须恰好一行：

```text
application=xcos
application_version=xcos-db-v1
schema_revision=1
schema_sha256=c648d0eb3dc04e3b32e774775072ba826c5a3f7b9945d306920f2f34f23223d0
```

xcss `_common_administrators` 表保存不透明 TEXT `administrator_id`、canonical `username`、密码摘要、启停状态、Session version 和微秒整数时间
字段，不保存 email 或 `role`。username 的 Schema CHECK 精确要求 3–64 bytes、ASCII 小写、首尾
`[a-z0-9]`、其余字符仅 `[a-z0-9._-]`；唯一索引直接作用于 canonical 值。登录 candidate 可包含首尾
ASCII whitespace/大写，但 xcss 规范化后才查询；`@`、Unicode、内部空白、控制字符和首尾分隔符
都会拒绝。所有成功认证的控制面账户都是 Administrator；禁止通过手改表或增加配置字段制造身份等级。

`media_reconciler_leases` 必须是当前固定结构且恰有 `singleton=1`。空闲 owner/expiry 同为 NULL；持有态
owner 是规范 UUIDv4，时间为 UTC RFC 3339 且 expiry 晚于 updated。产品不会修补非法状态。

所有敏感字段必须是当前规范 envelope，并能用当前 external key 解密。不要直接编辑数据库或复制密文
字段。

## 5. Doctor 与健康检查

```bash
set -a
source /etc/isarmg/xcos.env
set +a

"/opt/isarmg/xcos/releases/1.0.0/bin/xcos" doctor --offline
/opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
"/opt/isarmg/xcos/releases/1.0.0/bin/xcos" doctor
```

offline 检查 Schema、SQLite integrity/foreign keys、回滚写探针、录像目录、全量凭据解密、MediaMTX
binary/version/SHA/config。在线模式再检查两个 loopback readiness，并通过固定`/v3/config/pathdefaults/get`核对MediaMTX实际生效的`recordPath`。失败时先保全日志和状态，不能反复
启动掩盖 unknown operation。

录像目录只有`recordings_directory`（ENV为`RECORDINGS_DIR`）一个权威值。正式binary先验证完整物理发行树；仅当输入正是执行树内`config/mediamtx.yml`且字节等于编译期受管模板时，允许模板中生产路径的inert默认值由launcher的`MTX_PATHDEFAULTS_RECORDPATH`覆盖。native launcher从`RECORDINGS_DIR`构造该值；systemd的MediaMTX环境文件必须使用相同目录。有效目录必须为当前服务用户拥有的物理0700目录，不能使用符号链接或遍历路径。复制到外部或开发模式的YAML始终要求实际`recordPath`与目录匹配；改写或重复字段不属于受管模板。在线Doctor有3秒、16KiB、禁重定向的loopback API读取，拒绝companion实际目录与权威值不一致。

## 6. 锁顺序

应用全生命周期持有数据库 instance 排他、maintenance 共享和 runtime app lock。正常运行持有数据库父目录
`.state-maintenance.lock` 共享锁与 `.state-instance.lock` 独占锁；runtime 目录使用同一公共锁协议并维护 PID。
MediaMTX 由 `flock --no-fork` 持有 companion lock。维护工具必须按database maintenance -> runtime -> MediaMTX
取得排他锁。不要用不同 runtime 指向同一数据库；database identity lock 仍会拒绝第二实例。

## 8. 发布测试

```bash
for script in scripts/*.sh deploy/*.sh deploy/.*-action.sh deploy/xcosctl; do
  bash -n "$script" || exit 1
done
./scripts/lifecycle-test.sh
./scripts/relocated-smoke-test.sh
```

lifecycle test 仅使用临时根，覆盖 no-clobber、合同外环境拒绝、秘密不回显、失败回滚、start/stop
串行化及 symlink/hardlink 防御；relocated smoke 使用真实 Vite/Rust/SQLite，读取所有 hashed asset 并
证明篡改后拒绝重启。

## 9. 故障定位

| 现象 | 优先检查 |
|---|---|
| start 拒绝 | release manifest、权限、物理路径、MediaMTX SHA/config |
| 登录失败/循环 | 系统时钟、HTTPS、Secure Cookie、Origin/Host、限流 |
| operation 长期 pending | reconciler 日志、global/operation lease、MediaMTX API |
| operation unknown | 对照远端 actual state，禁止盲重试 |
| 无画面 | 摄像头 RTSP、publisher、JWT 时间窗、Caddy WHEP/HLS 路由 |
| doctor Schema 失败 | 停止服务，保全 generation 和错误记录 |
| 凭据解密失败 | 用受保护的当前 `CREDENTIALS_KEY` 核对授权码密文；不要自动换 key 或绕过认证 |

系统状态中的 `recording_configured` 只统计已启用且配置为服务器录像的摄像机，不证明 MediaMTX 正在持续写盘。媒体服务状态、流就绪、录像文件增长和磁盘错误需要分别观测。

管理页的录像摄像头选择限定为当前实例的服务器录像设备。实例或设备列表变化时，当前选择会指向仍可用的摄像头；重新查询录像后，播放选择会清空，避免把上一查询的片段显示为本次结果。

管理页“日志”使用服务器操作系统的本地日期。打开页面时从受保护的 `GET /api/v1/logs/calendar`
取得服务器当天；单日查询使用 `date=YYYY-MM-DD`，范围查询使用 `start_date=YYYY-MM-DD&end_date=YYYY-MM-DD`，
包含起止当天且不与 `date` 混用。事件、媒体协调操作和业务审计每页最多 100 条，
返回 `next_cursor`，游标绑定日期范围和记录类型；同一时间戳以记录 ID 排序，不遗漏后续页。
页面保留当前页及已访问的游标栈，支持首页、上一页和下一页；上一页不是服务端 `previous_cursor` 字段。每组响应上限 8 MiB，服务端响应等待最多 5 秒；每条历史 SQLite 连接最多等待 2 秒，查询执行在 3 秒进度截止时中断。全局最多 4 个查询，连接及响应 body 结束前持续占用名额，至少 6 个连接留给命令及状态。

新历史写入和媒体操作在同一写事务中检查持久预算：所有事件、审计、操作和 outbox 合计最多 100 万行，每摄像头最多 10 万行且记录内容、索引元数据计费与回执预留最多 2 GiB，数据库、WAL 与已接受操作后续回执/审计预留合计最多 8 GiB；新增操作还要求可用空间保留 1 GiB 加预留量。达到任一边界时返回明确的 `history_storage_capacity`，不接受新操作、不删除已有事实。已有警报确认、PTZ 回执、操作完成及人工结果确认使用预留空间继续处理。容量上限是拒绝条件，不是实测性能保证，也不等同于真实磁盘已满。
服务端按该日期的本地起点到次日本地起点
换算为 UTC 半开区间，夏令时切换日也按实际时长处理。日志行显示服务器当地时间及该条记录
对应的 UTC 偏移；日期归属以记录创建时间为准。服务端时区配置或系统时钟变化后，刷新日志页
可取得新的服务器当天。`GET /api/v1/events` 保留实时监控快照用途，日志读取使用
`GET /api/v1/events/logs`。需要核对较早创建的 Unknown/Failed 媒体操作时，选择其创建日期；
媒体调和与状态判断直接读取持久化状态，不以当前日志筛选结果为依据。

## 10. 安全事件

先隔离公网和摄像头 VLAN，停止扩大写入，保全数据库 generation、recordings、manifest、配置摘要、审计
与 Journal，再轮换 Session、管理员密码、JWT Secret、Credential Key、摄像头密码和 TLS 材料。不要在
公开 issue 上传数据库、录像、RTSP URL、账号、密钥或日志 Secret；使用私密漏洞报告渠道。只支持
当前发布版本与当前 `main`。

控制面认证只接受以下三个路径：

- `POST /api/v1/auth/login`；
- `GET /api/v1/auth/session`；
- `POST /api/v1/auth/logout`。

login/session 成功响应必须严格是
`{authenticated:true,user_id,username,role:"admin",csrf_token}`，登录请求必须严格为
`{username,password}`；额外字段、已删除的 email 字段、其他 `role` 值和合同外路径都视为
合同错误。wire 中固定的 `role:"admin"` 仅用于跨产品响应一致性，不对应数据库列。实例授权码与
Client token 是独立的设备数据面凭据，与 Administrator 身份无关。

`BOOTSTRAP_ADMIN_USERNAME`、管理 API 和内置 Web 是 Server 范围。Client 保管的设备账号、MediaMTX internal
auth body、媒体 JWT、WHEP/HLS 播放与录像状态不使用 Administrator username；运维轮换管理 username/
密码时不能同步改实例授权码或媒体 key，反之亦然。

## 11. Web 设计依赖与发布证明

当前 Web 使用 xcss 的 admin-web、admin-shell、admin-ui、contracts、design-tokens、http-client、
web-fonts、web-toolchain 八个内部模块，作为一个 @xcss/web 构建期包发布，不是生产运行服务。Node 固定为 `.node-version` 的 `26.7.0`。

- 候选Rust输入固定 xcss `=1.0.0` / `b0524c4fb018b5ba4f27ad71bf32b74c8ef0a972`，一个 @xcss/web 包使用对应新tag URL与真实tarball的lock integrity。xcss 1.0.0 已正式发布，官方单包已逐字节验证；产品仍须完成自身正式构建和发行验收。
- 旧消费者CI证明只属于其记录的旧revision；本次新源码和真实发行物必须分别验收。统一manifest、lockfile和发布身份，不改写旧tag/资产。

```bash
cd web
npm ci
npm run check:xcss
npm run build
npm run test:browser
```

`build` 自身以 `check:xcss` 为前置，不能绕开。精确工具链为 React/ReactDOM `19.3.0`、TypeScript
`7.0.2`、Vite `8.3.3`、`@vitejs/plugin-react` `6.1.2`、`@types/react` `19.3.0` 和
`@types/react-dom` `19.3.0`。Shell、登录/恢复/退出、通知、主题、诊断、Maple 字体和 UI 原语全部来自 xcss。
Xcos CSS 仅保留业务布局，不覆盖平台 token 或定义私有字体。HLS 按需单独加载，完整功能保留；各资产遵循
xcss 512 KiB 硬限制且不发布 source map。浏览器验收使用真实 dist，覆盖两种浏览器的系统/管理员、
摄像头、录像、事件、云台键盘停止、对话框焦点和移动明暗主题无障碍。后续
`scripts/build.sh` 把该 Hash 文件写入发行 manifest，`relocated-smoke-test.sh` 证明实际归档引用它并在篡改
后拒绝启动。生产目录中不应出现 `node_modules`、源包、`vendor/xcss-design` 或远程 CSS URL。

设计测试或依赖安装失败时，不要复制本地 `reset.css` 临时绕过，也不要在 `index.html` 添加 CDN。应修复
当前 xcss 来源/lockfile，重新生成整个 Web dist 和不可变 release。xcss 版本切换属于直接
替换当前合同，不保留并行 CSS 或媒体查询式版本 fallback。

系统页展示媒体状态与业务审计；管理员账号名称和密码从 Shell 右上角人物图标进入自助设置。
管理员 API 由 xcss 提供给后端集成，业务页面通过账号自助设置访问当前身份。
改密/停用撤销全部会话，安全审计与写入同事务。系统页展示的“业务审计”仍来自产品 `/audit`，与平台安全审计分工明确。

### 11.1 xcss 包的运维边界

| @xcss/web 内部模块入口 | Xcos 使用内容 | 运维必须证明 | 不由该包负责 |
|---|---|---|---|
| `@xcss/web/contracts` | auth 路径、Administrator DTO、ErrorEnvelope 类型与严格守卫 | 版本/来源锁定；不可信 JSON 通过守卫；未知字段拒绝 | 摄像头、录像、事件、审计等产品 DTO |
| `@xcss/web/http-client` | same-origin、Cookie、CSRF、超时、响应上限、Content-Type、错误解析 | unsafe 请求携带当前 CSRF；401 使本地 Session 失效；无跨 origin | 自动重试写操作、大文件下载、业务响应判定 |
| `@xcss/web/design-tokens` / `web-fonts` | token、scoped reset、Maple 字体 | `data-xcss-scope`、无私有字体覆盖、无 CDN | 业务布局 |
| `@xcss/web/admin-web` | API client、Session 状态机、管理 API client | 当前 auth/administrators 合同、401 generation、CSRF | 业务请求 DTO |
| `@xcss/web/admin-shell` / `admin-ui` | 登录、导航、诊断、主题、通知、管理员面板及交互原语 | 共享浏览器与消费者无障碍验收 | 摄像头、录像、事件业务 |
| `@xcss/web/web-toolchain` | Vite、strict tsconfig、精确工具链、资源预算和 source map 政策 | `check:xcss`、构建与独立发行验收 | 生产服务 |

### 11.2 xcss Rust 单体内部模块的运维边界

| 包内模块 | 共享能力 | Xcos 保留的产品责任 | 删除后果 |
|---|---|---|---|
| `xcss::contracts` | Administrator 路径/DTO、跨语言合同类型 | 业务 DTO | Rust/Web 认证合同可能静默漂移 |
| `xcss::admin_core` / `xcss::admin_sqlite` / `xcss::admin_axum` | 管理员、固定密码策略、Session、Cookie、CSRF、限流、事务审计与管理路由 | 启动时选择 Profile 并挂载平台 Router | 产品再次拥有第二套认证策略 |
| `xcss::error` | `ErrorCode`、严格 `ErrorEnvelope` | 业务状态映射和脱敏诊断 | 错误可能泄漏内部结构 |
| `xcss::schema_identity` / `xcss::platform_db` | metadata、当前平台表、fingerprint 和数据库初始化边界 | Xcos 业务 Schema、快照与全局调和租约不变量 | 平台 Schema 发生分叉 |
| `xcss::server_runtime` / `xcss::server_target` | 生命周期、任务监督、健康/诊断与正式 target | 注册业务任务、业务 Router、MediaMTX 伴随进程合同 | 生命周期与运行目标漂移 |

这些共享包不提供旧合同兼容。升级包版本时同时替换依赖、lockfile、代码消费者、检查和整套发行物，
不在 Xcos 内加入双读、别名或 fallback。

## 开发构建与 Web 热更新

```bash
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/xcss-web-build.json" --mode development --no-install
```

按当前配置解析器设置实验用数据库、凭据和 MediaMTX 环境，使用 `APP_ENV=development` 和回环绑定，先从stdin读取本地管理员密码执行`init --username admin`，再运行
`target/x86_64-unknown-linux-gnu/debug/xcos run`。默认使用这次构建内嵌的资源。需要目录热更新时，显式设置
`XCSS_DEV_WEB_DIR="$PWD/web/dist"`，重新构建 Web 后 HTTP 就能读取新内容，无需重编 Rust；也可以
在 `web` 运行 Vite，保持已有 `/api/v1` 和媒体代理。只有未绑定开发 binary 接受此选择，生产
`APP_ENV=production` 与正式 `run --release-root` 都拒绝目录覆盖。未知资源返回 404，HTML 不缓存，其他
资源通过 SHA-256 ETag 校验缓存。

## 当前中立接口与旧版数据处理

当前版本只使用 `.state-instance.lock`、`.state-maintenance.lock`、`.state-maintenance-pending.json` 和 `.state-atomic-` 临时文件前缀；离线升级工具采用 `.release-upgrade` 工作目录。服务身份头为 `x-service`，健康状态中的公共源码修订字段为 `common_revision`。管理会话采用 `__Host-admin-xcos-session`，显式开发模式采用 `admin-xcos-session`；生产 Cookie 的 Secure、HttpOnly、SameSite、Path 和 CSRF 约束继续生效。资源清单格式为 `web-assets-v1`，公共数据库内部表及索引采用 `_common_` 前缀。

这些接口没有旧名称别名或旧版兼容分支。旧版升级前，先按本文的停服步骤停止服务、配套客户端及全部维护工具；确认全部进程退出后，完整备份配置、SQLite 数据库及其 WAL/SHM、业务文件和必要的私有凭据。备份包含敏感数据，应保留原有访问权限并离线保存。

保留旧数据目录，按当前安装步骤配置新的私有数据目录，执行显式 `init` 初始化，随后运行 `config validate`，再启动服务、登录管理页面并重新配对客户端。旧配置应人工审阅后填写当前字段，不能整体覆盖新目录。旧业务数据需要另行处理；当前版本不提供自动迁移。不得让旧、新版本同时写同一目录，不得通过删锁文件或修改数据库 metadata 强制启动；当前结构指纹包含实际表名、索引名和 SQL，仅改名称不能证明数据符合当前合同。

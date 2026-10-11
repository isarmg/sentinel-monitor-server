# xcos 开发与验证

Rust/Axum 控制面按 CLI、服务组装、数据库和 HTTP 业务职责分工。当前使用 Rust 1.99.0、SQLx 0.9；
Web 使用 Node.js 26.7.0。构建和运行目标固定 Linux AMD64 GNU。当前源码和公开历史发行包分别按自身
数据库合同运行；不要把发布成功视为其后的代码已包含在同名发行资产中。

## 开发验证

```sh
cargo +1.99.0 fmt --all -- --check
cargo +1.99.0 clippy --locked --all-targets -- -D warnings
cargo +1.99.0 test --locked --all-features
(cd web && npm ci && npm run build)
./scripts/lifecycle-test.sh
```

发行包、MediaMTX lock、私有配置、初始化与端口要求见[运维文档](operations.md)。设备侧安装、
配对、服务管理、诊断与卸载见 [xcoc 平台指南](https://github.com/isarmg/xcoc/blob/main/docs/platform-setup.md)。

## 协议与初始化边界

核心 CLI 为 `init`、`run`、`config validate` 和 `status`；`--config` 读取私有 JSON，显式 CLI
覆盖环境、文件和默认值，`--json` 提供单条机器错误。普通运行只接受当前格式，非当前输入拒绝且不改写。

当前数据格式为 `xcos-db-v2`（Schema 2），Client 协议为 `xcos-edge-v1`，能力采用
`supported/unsupported/unknown` 三态；浏览器 wire 身份为 `xcos-wire-v2`（HTTP 前缀仍为
`/api/v1`），媒体 JWT 为 v1。这些身份独立管理，不可互换。

状态读取失败时的 `fresh/stale/unknown` 观测合同及安全要求见[运行状态参考](runtime-reference.md#摄像机状态与观测时效)。
许可与第三方组件信息以发行包内的许可证清单为准。


## 构建正式归档

Linux AMD64 GNU 构建机准备 Rust 1.99.0、Node.js 26.7.0、C 工具链、Python 3.11+、curl、OpenSSL 和 GNU 工具。正式打包要求干净源码，且对应 annotated tag 精确指向 HEAD；同名旧标签不代表当前源码已发布。

```bash
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-gnu
output="$(mktemp -d /var/tmp/xcos-release.XXXXXXXX)"
bash scripts/package-release.sh "$output"
```

脚本下载并核对固定的 MediaMTX 1.20.0，构建内嵌 Web 的程序，生成完整发行树、归档和 SHA256SUMS。输出目录位于源码树外，由构建用户拥有且其他用户不可写；成功产物按[安装指南](getting-started.md)使用。

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

## 11. Web 设计依赖与发布证明

当前 Web 使用 xcss 的 admin-web、admin-shell、admin-ui、contracts、design-tokens、http-client、
web-fonts、web-toolchain 八个内部模块，作为一个 @xcss/web 构建期包发布，不是生产运行服务。Node 固定为 `.node-version` 的 `26.7.0`。

- 候选Rust输入固定 xcss `=1.0.2` / `3f751196615edd9f7fda2d76a5aa90f9f42586dc`，一个 @xcss/web 包使用对应新tag URL与真实tarball的lock integrity。xcss 1.0.2 已正式发布，官方单包已逐字节验证；产品仍须完成自身正式构建和发行验收。
- 针对本次最终源码和最终发行物分别保存验证结果，统一 manifest、lockfile 和发布身份。

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

更新公共包时，同步依赖、lockfile、调用代码、检查和发行物，并核对当前接口身份。

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

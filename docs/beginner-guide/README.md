# 从零读懂 xcos

面向第一次接触 Rust Web 与摄像头媒体链路的开发者。想先用起来，请从[安装指南](../getting-started.md)和[摄像头操作](../usage.md)开始；本教程用于解释它们背后的实现。

## 阅读顺序

1. [项目全景与版本边界](01-project-overview.md)
2. [开发环境与第一次运行](02-environment-and-first-run.md)
3. [Rust、视频链路与 Web 基础](03-rust-media-and-web-basics.md)
4. [服务端请求、认证与摄像头管理](04-server-request-and-camera-lifecycle.md)
5. [持久操作、协调器与故障恢复](05-operations-reconciler-and-recovery.md)
6. [MediaMTX、录像与播放链路](06-mediamtx-recording-and-playback.md)
7. [当前协议、加密与状态合同](07-current-contracts-and-cryptography.md)
8. [测试、调试与变更方法](08-testing-debugging-and-change-workflow.md)
9. [部署、安全与生产运维](09-deployment-security-and-operations.md)
10. [源码路线、练习与术语表](10-reading-roadmap-and-glossary.md)

## 按任务选读

- 配置并运行开发环境：第 1、2、8 章
- 修改管理页面或 API：第 3、4、7、8 章
- 排查画面、录像或操作状态：第 5、6、9 章
- 理解版本、加密与状态身份：第 7 章和[运行参考](../runtime-reference.md)

每章包含实现入口和练习。使用独立测试数据，完成一次构建和测试后，再追踪相应源码。术语见第 10 章，生产操作统一见[运维](../operations.md)。

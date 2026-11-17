# Google 登录

普通用户从「导入歌单 → YouTube Music → 使用 Google 登录」授权，无需下载或选择 OAuth JSON。首页保留档案视觉，检索与导入使用顶栏常规入口。

## 应用身份与账号授权

发布者必须在属于 Nasti 的 Google Cloud 项目中启用 YouTube Data API v3，注册 Desktop app OAuth 客户端，配置应用品牌、同意页面及请求的歌单权限。公开发行前，按 Google 对生产应用和敏感权限的要求完成相应审核。这是应用身份的前置条件，与用户随后选择账号、同意权限是两件事。

构建时通过环境变量 `NASTI_APP_YOUTUBE_CLIENT_ID` 提供 Nasti 专用的公开客户端 ID：

```sh
NASTI_APP_YOUTUBE_CLIENT_ID='<Nasti Desktop client ID>' pnpm build:mac
```

只将公开客户端 ID 编译进原生宿主。此路径不读取客户端 JSON、不嵌入 client_secret，也不包含账号、refresh token 或本机配置。Google 当前桌面授权文档将 client_secret 标为可选；授权使用 PKCE。缺少 ID 的源码构建仍可使用曲库与导入功能，Google 登录按钮禁用并显示「此版本尚未启用 Google 登录」。代码仓库不提供或借用第三方客户端身份。

为兼容已有安装，显式的运行时开发覆盖 `NASTI_YOUTUBE_CLIENT_ID` / `NASTI_YOUTUBE_CLIENT_FILE` 以及原数据目录中的既有客户端配置继续可读；既有本地配置优先于构建 ID，以保留其客户端对应的 Keychain 授权。普通界面不再提供 JSON 选取。不得把用户私人客户端文件恢复到公开包。

macOS 的 refresh token 延续现有 Keychain 保存方式。使用 `NASTI_DATA_DIR` 的隔离验收不读写正式 Keychain 授权，授权仅在当前进程内保留。正式曲库、Vault 与更新信任根不随本次改动迁移。

## 浏览器与取消

Google 官方禁止把 OAuth 请求交给开发者可控制的嵌入式用户代理，包括可注入脚本或访问会话 cookie 的 WebView。因此本实现使用系统浏览器、随机状态、PKCE S256、随机本机 loopback 端口，并验证回调状态；不会伪装浏览器或提取 Google cookie。

等待用户授权时可选择「取消登录」或按 Esc。离开面板也会取消等待，释放本机回调端口；重试创建新的状态与 PKCE。Google 已返回有效授权码时，完成交换与保存，界面显示实际连接结果，以免隐藏已经保存的授权。关闭浏览器页本身不会通知应用，用户可在应用取消或等待超时。

## 官方依据

核对日期：2026-11-17。

- [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)：桌面公共客户端、PKCE、系统浏览器、本机回调、可选 client_secret。
- [OAuth 政策：注册客户端](https://developers.google.com/identity/protocols/oauth2/policies#register)。
- [OAuth 政策：安全浏览器](https://developers.google.com/identity/protocols/oauth2/policies#browsers)。
- [OAuth 政策：生产应用审核](https://developers.google.com/identity/protocols/oauth2/policies#submit-for-verification)。

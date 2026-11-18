# Google 登录

普通用户从「导入歌单 → YouTube Music → 使用 Google 登录」授权，无需下载或选择 OAuth JSON。首页保留档案视觉，检索与导入使用顶栏常规入口。

首次打开 YouTube Music 面板时，先阅读并同意 Nasti 的隐私政策与条款。应用记录本次同意，之后在面板中保留文档链接。三份公开页面由仓库的 `gh-pages` 分支发布。

[Nasti 官网](https://xiaotwu.github.io/Nasti/) · [隐私政策](https://xiaotwu.github.io/Nasti/privacy.html) · [使用条款](https://xiaotwu.github.io/Nasti/terms.html)

## 应用身份与账号授权

发布者必须在属于 Nasti 的 Google Cloud 项目中启用 YouTube Data API v3，注册 Desktop app OAuth 客户端，配置应用品牌、同意页面及请求的歌单权限。公开发行前，按 Google 对生产应用和敏感权限的要求完成相应审核。这是应用身份的前置条件，与用户随后选择账号、同意权限是两件事。

构建时通过环境变量提供 Nasti 专用 Desktop 客户端的应用身份：

```sh
NASTI_APP_YOUTUBE_CLIENT_ID='<Nasti Desktop client ID>' \
NASTI_APP_YOUTUBE_CLIENT_SECRET='<Nasti application client secret>' pnpm build:mac
```

只将 Nasti 自身的应用凭据编译进原生宿主，不打包客户端 JSON、账号、refresh token 或本机配置。凭据由发布者从源码树外的受限文件或密钥存储读入构建进程环境，不写入公开仓库。

Google 当前桌面文档将 client_secret 标为可选，但 2026-11-17 对本次专用 Desktop 客户端的实际 token 请求返回 `invalid_request / client_secret is missing.`；提供应用 client_secret 后，测试无效码的响应推进为 `invalid_grant / Malformed auth code.`。因此发布构建要求 ID 和应用 secret 同时提供，缺失时构建失败。普通用户不承担这个配置步骤。

桌面应用是公共客户端，分发的原生二进制不能保守应用 client_secret 的机密性，它不是账号访问授权；PKCE 和随机状态仍是必要保护。用户令牌只在授权后的当前进程内使用。缺少应用 ID 的源码构建仍可使用曲库与导入，Google 登录按钮禁用并显示「此版本尚未启用 Google 登录」。代码仓库不提供或借用第三方客户端身份。

为兼容已有安装，显式的运行时开发覆盖 `NASTI_YOUTUBE_CLIENT_ID` / `NASTI_YOUTUBE_CLIENT_FILE` 以及原数据目录中的既有客户端配置继续可读；既有本地客户端配置继续优先于内置客户端。普通界面不再提供 JSON 选取。不得把用户私人客户端文件恢复到公开包。

## 授权检查与本地清理

访问令牌、刷新令牌、频道和歌单快照只保留在当前进程内。退出再打开应用后，需要重新连接并导入。当前进程大约每 15 分钟尝试刷新授权。Google 返回无效授权时清除本地数据。暂时无法确认时保留会话授权，超过一小时未验证即清除歌单数据。持续运行达到 24 小时时，歌单数据由周期检查或下次请求清除。休眠期间无法联网检查，恢复运行后按已过去的时间清理。

在应用中选「撤销授权」会先清除歌单，并立即向 Google 发出撤权请求。暂时失败时，应用运行期间会重试。退出后可以在 [Google 账号权限](https://myaccount.google.com/permissions) 确认或移除访问。移除本地记录会保留 YouTube 上的歌单。删除在线歌单是独立操作。

首次启动会清理 SQLite 主库和 WAL 中旧版 YouTube 快照，以及旧版 Nasti 专用 Keychain 服务的用户授权。本地曲库、Vault 和客户端配置继续可用。用户或系统另存的历史备份由其原管理工具控制。隔离验收用 `NASTI_DATA_DIR`，避免接触正式 Keychain 数据。

## 浏览器与取消

Google 官方禁止把 OAuth 请求交给开发者可控制的嵌入式用户代理，包括可注入脚本或访问会话 cookie 的 WebView。因此本实现使用系统浏览器、随机状态、PKCE S256、随机本机 loopback 端口，并验证回调状态；不会伪装浏览器或提取 Google cookie。

等待用户授权时可选择「取消登录」或按 Esc。离开面板也会取消等待，释放本机回调端口；重试创建新的状态与 PKCE。Google 已返回有效授权码时，完成交换并显示实际连接结果。关闭浏览器页本身不会通知应用，用户可在应用取消或等待超时。

## 官方依据

核对日期：2026-11-17。

- [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)：桌面公共客户端、PKCE、系统浏览器、本机回调；client_secret 的文档描述与本次实际服务响应差异见上文。
- [OAuth 政策：注册客户端](https://developers.google.com/identity/protocols/oauth2/policies#register)。
- [OAuth 政策：安全浏览器](https://developers.google.com/identity/protocols/oauth2/policies#browsers)。
- [OAuth 政策：生产应用审核](https://developers.google.com/identity/protocols/oauth2/policies#submit-for-verification)。

# Google 登录与歌单同步

从右上角账号图标打开登录卡片，在系统浏览器完成 Google OAuth 授权。未配置发行者客户端的构建可在卡片中导入自己从 Google Cloud 下载的 Desktop app 客户端 JSON。需要启用 YouTube Data API v3。客户端配置仅用于本机，OAuth 回调使用回环地址、PKCE 和随机状态。

访问令牌保留在进程内。刷新令牌与频道 ID 保存在应用数据目录的受保护会话文件中，macOS 另尝试保存刷新令牌到钥匙串。重启从该文件恢复，后台刷新访问令牌。自有与公开订阅的歌单快照保存在本机 SQLite 数据库。临时连接失败时仍可查看上次同步的快照。快照须在 30 天内刷新，过期记录由应用启动和运行时清理。

账号气泡可手动同步或断开连接。YouTube 确认撤权后，Nasti 清除该账号的歌单数据。公开订阅可在曲库总览单独取消，Vault 离线副本可在设置中移除。已下载的副本由 Vault 单独管理。

发行包必须使用空曲库，排除账号凭证、客户端 JSON、SQLite 和 Vault。验收可设定独立的 `NASTI_DATA_DIR`。发行者桌面客户端由构建环境提供。

- [Google 桌面授权](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Nasti 隐私政策](https://xiaotwu.github.io/Nasti/privacy.html)
- [YouTube API 开发者政策](https://developers.google.com/youtube/terms/developer-policies)

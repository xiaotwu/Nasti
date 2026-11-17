import { t } from "./i18n";

// The desktop commands and browser mock return strings today. Translate their stable
// message text at the UI boundary, keeping operating-system and service prose out of it.
const messages = new Map<string, string>([
  ["授权曲库暂时无法打开", "The authorized library could not be opened."],
  ["授权媒体暂时无法准备", "The authorized media could not be prepared."],
  ["此版本没有可播放的授权文件", "This version has no authorized file to play."],
  ["请先导入此曲所属的歌单", "Import this track’s playlist first."],
  ["授权曲库中没有此曲目", "This track is missing from the authorized library."],
  ["授权曲库中没有此歌单", "This playlist is missing from the authorized library."],
  ["请输入目录中的歌单 ID 或 nasti://playlist/ 链接", "Enter a catalog playlist ID or a nasti://playlist/ link."],
  ["请在 Nasti 桌面版使用在线曲库", "Open the online library in Nasti for desktop."],
  ["源曲目已变化", "The source track has changed."],
  ["没有此歌单", "This playlist could not be found."],
  ["曲库中没有此媒体版本", "This media version is missing from the library."],
  ["曲库中没有这个媒体版本", "This media version is missing from the library."],
  ["无法读取授权媒体", "The authorized media could not be read."],
  ["上限必须介于当前用量与 2 GiB 之间", "The limit must be between the current usage and 2 GiB."],
  ["无法打开授权媒体目录", "The authorized media catalog could not be opened."],
  ["授权媒体目录格式错误", "The authorized media catalog has an invalid format."],
  ["授权媒体目录记录无效", "The authorized media catalog contains an invalid record."],
  ["授权媒体目录中没有此曲目", "This track is missing from the authorized media catalog."],
  ["所选曲目没有此媒体版本", "This version of the selected track is unavailable."],
  ["授权曲目缺少有效媒体地址", "This authorized track has no valid media URL."],
  ["目录为空", "The catalog is empty."],
  ["曲库数据库操作失败", "The library database operation failed."],
  ["已保存的在线曲目信息无效", "Saved online track information is invalid."],
  ["未知歌单。可导入内置歌单链接或 Internet Archive 专辑", "Playlist not found. You can import a built-in playlist link or an Internet Archive album."],
  ["请选择第 1 至第 9 列", "Choose a column between 1 and 9."],
  ["该列已有歌单", "This column already has a playlist."],
  ["九列均已使用", "All nine columns are in use."],
  ["曲目入列记录已变化，请重新选择", "The playlist entries have changed. Select them again."],
  ["曲库正在另一个 Nasti 窗口中使用", "Another Nasti window is using the library."],
  ["导入预览已过期，请重新查找歌单", "The import preview has expired. Find the playlist again."],
  ["这首授权曲目仅提供音频", "Only audio is available for this authorized track."],
  ["请使用 Internet Archive 的 /details/ 专辑链接", "Use an Internet Archive /details/ album link."],
  ["该专辑缺少 unfoundsound 厂牌记录、可用的许可或公开 MP3", "This album is missing an unfoundsound label record, an eligible license, or a public MP3."],
  ["Internet Archive 请求过多，请稍后重试", "Internet Archive received too many requests. Try again later."],
  ["无法连接 Internet Archive", "Internet Archive could not be reached."],
  ["Internet Archive 返回的目录数据无效", "Internet Archive returned invalid catalog data."],
  ["Internet Archive 曲目文件信息无效", "Internet Archive track file information is invalid."],
  ["无法连接 Internet Archive 音源", "The Internet Archive audio source could not be reached."],
  ["Internet Archive 未返回与已登记曲目匹配的音频", "Internet Archive did not return audio matching the registered track."],
  ["Vault 存储操作失败", "The Vault storage operation failed."],
  ["无效的曲目 ID", "The track ID is invalid."],
  ["Vault 已达存储上限", "Vault has reached its storage limit."],
  ["Vault 已达存储上限。可移除下载，或在设置中提高上限", "Vault has reached its storage limit. Remove a download or raise the limit in Settings."],
  ["存储上限不得超过 2 GiB", "The storage limit cannot exceed 2 GiB."],
  ["存储上限不得小于已下载大小", "The storage limit cannot be below the space already in use."],
  ["Vault 暂时不可用，请重启应用", "Vault is unavailable. Restart the app and try again."],
  ["下载的大小或校验和与授权曲目记录不符", "The download size or checksum does not match the authorized track."],
  ["原生播放桥已关闭", "The native playback connection is closed."],
  ["原生播放状态读取失败", "Native playback status could not be read."],
  ["授权曲目地址无效", "The authorized track URL is invalid."],
  ["曲目标识无效", "The track identifier is invalid."],
  ["已切换到另一首曲目", "Playback has switched to another track."],
  ["媒体切换已取消", "The media switch was canceled."],
  ["媒体暂时无法准备", "The media could not be prepared."],
  ["媒体准备超时", "The media took too long to prepare."],
  ["媒体切换标识无效", "The media switch identifier is invalid."],
  ["媒体还未就绪", "The media is not ready yet."],
  ["音量值无效", "The volume is invalid."],
  ["无法读取播放设备", "Could not read playback devices."],
  ["播放设备已断开", "The playback device disconnected."],
  ["播放设备标识无效", "The playback device identifier is invalid."],
  ["视频画面仅支持主窗口", "Video is only available in the main window."],
  ["视频画面坐标无效", "The video frame coordinates are invalid."],
  ["原生视频画面不可用", "Native video is unavailable."],
  ["此版本的原生播放桥支持 macOS", "This version supports native playback on macOS."],
  ["媒体播放失败", "The media could not be played."],
  ["请输入 YouTube Music 或 YouTube 的歌单链接或 ID", "Enter a YouTube Music or YouTube playlist link or ID."],
  ["请输入 YouTube Music 或 YouTube 的曲目链接或视频 ID", "Enter a YouTube Music or YouTube track link or video ID."],
  ["歌单名称不能为空", "Enter a playlist name."],
  ["请先连接拥有该歌单的 YouTube 频道", "Connect the YouTube channel that owns this playlist."],
  ["此 Google 帐号没有可用的 YouTube 频道", "This Google account has no available YouTube channel."],
  ["授权返回了多个 YouTube 频道，请重新连接需要管理的频道", "Authorization returned multiple YouTube channels. Reconnect with the channel you want to manage."],
  ["找不到该 YouTube 歌单，或当前帐号没有读取权限", "This YouTube playlist could not be found or is unavailable to the current account."],
  ["找不到该入列记录，请刷新歌单后重试", "The playlist entry could not be found. Refresh the playlist and try again."],
  ["YouTube 未提供此条目的视频 ID，当前无法移动该条目", "YouTube did not provide a video ID, so this entry cannot be moved."],
  ["曲目位置无效，请刷新歌单后重试", "The track position is invalid. Refresh the playlist and try again."],
  ["请在 YouTube 的歌单设置中把排序方式改为手动，再移动曲目", "Set the playlist to manual order on YouTube, then move the track again."],
  ["YouTube Data API 不支持对这个歌单执行该操作", "The YouTube Data API does not support this action for this playlist."],
  ["YouTube 请求已达到配额或速率限制，请稍后重试", "YouTube reached its quota or request limit. Try again later."],
  ["YouTube 授权无效，请重新连接帐号", "YouTube authorization is invalid. Reconnect your account."],
  ["读取过程中歌单条目已变化，请重新刷新", "Playlist entries changed while loading. Refresh the playlist."],
  ["YouTube 返回的数据格式无效", "YouTube returned invalid data."],
  ["无法连接 YouTube Data API", "The YouTube Data API could not be reached."],
  ["YouTube 已保存更改，列表暂未更新。请刷新歌单确认。", "YouTube saved the changes. Refresh the playlist to see them."],
  ["YouTube 已保存更改，本地更新失败", "YouTube saved the changes, but the local update failed."],
  ["YouTube 已删除歌单，本地副本清理失败", "YouTube deleted the playlist, but the local copy could not be removed."],
  ["请先在 YouTube Music 面板连接 Google 账号", "Connect a Google account in the YouTube Music panel first."],
  ["无法初始化 Google 授权连接", "Google authorization could not be initialized."],
  ["尚未配置 Google 桌面 OAuth 客户端", "Set up the Google desktop OAuth client first."],
  ["无法打开本机授权回调", "The local authorization callback could not be opened."],
  ["无法读取本机授权回调地址", "The local authorization callback address could not be read."],
  ["Google 授权码已过期，请重新连接", "The Google authorization code has expired. Reconnect your account."],
  ["Google 未返回持久授权，请重新同意歌单权限", "Google did not provide lasting authorization. Grant playlist access again."],
  ["暂时无法联系 Google 撤销授权，请重试", "Google could not be reached to revoke access. Try again."],
  ["Google 未能确认授权已撤销，请重试", "Google could not confirm that access was revoked. Try again."],
  ["Google 授权已撤销。无法移除钥匙串条目，请重试移除", "Google access was revoked, but the keychain entry could not be removed. Try removing it again."],
  ["无法读取 Google 授权", "Google authorization could not be read."],
  ["无法保存 Google 授权", "Google authorization could not be saved."],
  ["无法移除 Google 授权", "Google authorization could not be removed."],
  ["暂时无法连接 Google 授权服务", "Google’s authorization service could not be reached."],
  ["Google 返回的授权信息无法读取", "Google returned unreadable authorization data."],
  ["请检查 Google 桌面 OAuth 客户端配置", "Check the Google desktop OAuth client configuration."],
  ["找不到 Google 桌面 OAuth 客户端 JSON", "The Google desktop OAuth client JSON file could not be found."],
  ["无法读取 Google 桌面 OAuth 客户端 JSON", "The Google desktop OAuth client JSON file could not be read."],
  ["Google 桌面 OAuth 客户端 JSON 文件无效", "The Google desktop OAuth client JSON file is invalid."],
  ["请选择从 Google Cloud 下载的 Desktop app OAuth 客户端 JSON", "Select the desktop application’s OAuth client JSON downloaded from Google Cloud."],
  ["Google 桌面 OAuth 客户端 JSON 缺少客户端 ID", "The Google desktop OAuth client JSON file is missing its client ID."],
  ["Google 尚未授予歌单权限，请重新连接", "Google has not granted playlist access. Reconnect your account."],
  ["本机授权回调已关闭", "The local authorization callback is closed."],
  ["等待 Google 授权超时，请重试", "Google authorization timed out. Try again."],
  ["未授予 Google 歌单权限", "Google playlist access was not granted."],
  ["无法打开此 YouTube 链接", "This YouTube link could not be opened."],
  ["无法打开系统浏览器", "The default browser could not be opened."],
  ["此宿主尚未接入系统浏览器", "This host cannot open the default browser."],
  ["钥匙串中的 Google 授权记录无效", "The Google authorization record in the keychain is invalid."],
  ["无法读取钥匙串中的 Google 授权", "Google authorization could not be read from the keychain."],
  ["无法将 Google 授权保存在钥匙串", "Google authorization could not be saved to the keychain."],
  ["无法移除钥匙串中的 Google 授权", "Google authorization could not be removed from the keychain."],
]);
const chineseByEnglish = new Map(Array.from(messages, ([zh, en]) => [en, zh]));

export function localizeError(
  error: unknown,
  zhFallback = "操作暂时无法完成，请重试。",
  enFallback = "The action could not be completed. Try again.",
): string {
  const message = typeof error === "string" ? error.trim() : error instanceof Error ? error.message.trim() : "";
  const direct = messages.get(message);
  if (direct) return t(message, direct);
  const chinese = chineseByEnglish.get(message);
  if (chinese) return t(chinese, message);

  const readback = /^YouTube 已保存更改，歌单刷新失败。可重新读取歌单 ([A-Za-z0-9_-]+)。/.exec(message);
  if (readback) {
    return t(
      `YouTube 已保存更改。请重新读取歌单 ${readback[1]} 确认结果。`,
      `YouTube saved the changes. Reload playlist ${readback[1]} to confirm them.`,
    );
  }
  const service = /^(YouTube 拒绝了请求|Google 授权请求失败)（HTTP ([0-9]{3})(?:，[^）]*)?）$/.exec(message);
  if (service) {
    const english = service[1].startsWith("YouTube") ? "YouTube rejected the request" : "Google authorization failed";
    return t(`${service[1]}（HTTP ${service[2]}）`, `${english} (HTTP ${service[2]}).`);
  }

  const domainMessage = message.split(/[：（]/, 1)[0];
  const prefix = messages.get(domainMessage)
    ?? (message.startsWith("YouTube 已保存更改，本地更新失败。") ? messages.get("YouTube 已保存更改，本地更新失败") : undefined)
    ?? (message.startsWith("YouTube 已删除歌单，本地副本清理失败。") ? messages.get("YouTube 已删除歌单，本地副本清理失败") : undefined);
  if (prefix) return t(chineseByEnglish.get(prefix) ?? domainMessage, prefix);
  return t(zhFallback, enFallback);
}

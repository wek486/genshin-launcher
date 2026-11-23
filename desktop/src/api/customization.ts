import { convertFileSrc, invoke } from '@tauri-apps/api/core'

export type MediaSlot = 'icon' | 'wallpaper'

interface LocalMedia {
  path: string
  kind: 'image' | 'video' | 'audio'
}

export interface GameWallpaper {
  url: string
  kind: 'image' | 'video'
}

export interface VideoCrop {
  width: number
  height: number
  x: number
  y: number
}

export interface GameStatus {
  executablePath: string | null
  available: boolean
  running: boolean
}

function toMedia(media: LocalMedia | null): GameWallpaper | null {
  return media && media.kind !== 'audio' ? { url: convertFileSrc(media.path), kind: media.kind } : null
}

async function loadMusic(command: 'get_media' | 'choose_media') {
  const media = await invoke<LocalMedia | null>(command, { slot: 'music' })
  return media?.kind === 'audio' ? convertFileSrc(media.path) : null
}

/** 本机原神入口设置和进程管理。文件路径由系统选择器确认并持久化。 */
export const gameApi = {
  status: () => invoke<GameStatus>('game_status'),
  choosePath: () => invoke<string | null>('choose_game_path'),
  launch: () => invoke<void>('launch_game'),
}

/** 图标和壁纸存放在应用数据目录，URL 由 Tauri 的资源协议提供。 */
export const mediaApi = {
  async get(slot: MediaSlot) {
    return toMedia(await invoke<LocalMedia | null>('get_media', { slot }))
  },
  async choose(slot: MediaSlot) {
    return toMedia(await invoke<LocalMedia | null>('choose_media', { slot }))
  },
  async importWorkshop(item: string) {
    return toMedia(await invoke<LocalMedia>('import_workshop_wallpaper', { item }))
  },
  videoCrop: () => invoke<VideoCrop | null>('wallpaper_video_crop'),
  reset: (slot: MediaSlot) => invoke<void>('reset_media', { slot }),
}

export const musicApi = {
  get: () => loadMusic('get_media'),
  choose: () => loadMusic('choose_media'),
  remove: () => invoke<void>('reset_media', { slot: 'music' }),
}

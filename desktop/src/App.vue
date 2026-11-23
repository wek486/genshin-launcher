<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Minus, Music2, Play, Settings2, Square, VolumeX, X } from 'lucide-vue-next'
import bundledIcon from '../src-tauri/icons/128x128.png'
import GameBackdrop from './components/GameBackdrop.vue'
import { gameApi, mediaApi, musicApi, type GameStatus, type GameWallpaper, type MediaSlot } from './api/customization'
import './style.css'

const desktopWindow = isTauri() ? getCurrentWindow() : undefined
const defaultCover = '/art/genshin.jpg'
const backgroundFitKey = 'launcher.backgroundFit'
const musicMutedKey = 'launcher.musicMuted'
type BackgroundFit = 'cover' | 'contain'

function readPreference(key: string) {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function savePreference(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // The current window can still apply the setting if storage is unavailable.
  }
}

const game = ref<GameStatus>({ executablePath: null, available: false, running: false })
const icon = ref<GameWallpaper | null>(null)
const wallpaper = ref<GameWallpaper | null>(null)
const backgroundFit = ref<BackgroundFit>(readPreference(backgroundFitKey) === 'contain' ? 'contain' : 'cover')
const musicUrl = ref<string | null>(null)
const musicMuted = ref(readPreference(musicMutedKey) === 'true')
const musicElement = ref<HTMLAudioElement>()
const musicError = ref('')
const iconUrl = computed(() => icon.value?.url ?? bundledIcon)
const loading = ref(Boolean(desktopWindow))
const busy = ref(false)
const message = ref('')
const settingsError = ref('')
const workshopItem = ref('')
const importingWorkshop = ref(false)
const settings = ref<HTMLDialogElement>()
const trayVisible = ref(true)
const webviewVisible = ref(!document.hidden)
const mediaActive = computed(() => trayVisible.value && webviewVisible.value)
const musicActive = computed(() => mediaActive.value && !game.value.running)
const musicButtonTitle = computed(() => {
  if (!musicUrl.value) return '设置背景音乐'
  return musicMuted.value ? '恢复背景音乐' : '静音背景音乐'
})
let mounted = false
let stopVisibility: (() => void) | undefined
let statusTimer: ReturnType<typeof setInterval> | undefined

const statusText = computed(() => {
  if (!desktopWindow) return '在 Tauri 桌面端配置和启动游戏'
  if (loading.value) return '正在读取本地设置'
  if (game.value.running) return '游戏程序正在运行'
  if (game.value.available) return '已配置游戏路径'
  return game.value.executablePath ? '找不到游戏程序，请重新选择' : '请先选择已安装的游戏程序'
})

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : '操作失败，请重试。'
}

watch(iconUrl, (url) => {
  const favicon = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
  if (favicon) {
    favicon.removeAttribute('type')
    favicon.href = url
  }
})

watch(backgroundFit, (fit) => savePreference(backgroundFitKey, fit))
watch(musicMuted, (muted) => savePreference(musicMutedKey, String(muted)))

watch([musicElement, musicUrl, musicMuted, musicActive], ([element, url, muted, active]) => {
  if (!element || !url) return
  if (muted || !active) {
    element.pause()
    return
  }
  void element.play().catch((error: unknown) => {
    if (musicUrl.value !== url || musicMuted.value || !musicActive.value) return
    if (error instanceof DOMException && error.name === 'AbortError') return
    musicError.value = '背景音乐无法播放，请重新选择音频文件。'
  })
}, { flush: 'post' })

function syncVisibility() {
  webviewVisible.value = !document.hidden
}

onMounted(async () => {
  mounted = true
  document.addEventListener('visibilitychange', syncVisibility)
  if (!desktopWindow) return
  void desktopWindow.listen<boolean>('launcher-visibility', ({ payload }) => {
    trayVisible.value = payload
  }).then((unlisten) => {
    if (mounted) stopVisibility = unlisten
    else unlisten()
  }).catch((error) => { message.value = errorMessage(error) })

  const results = await Promise.allSettled([
    gameApi.status(), mediaApi.get('icon'), mediaApi.get('wallpaper'), musicApi.get(),
  ] as const)
  if (!mounted) return
  const [savedGame, savedIcon, savedWallpaper, savedMusic] = results
  if (savedGame.status === 'fulfilled') game.value = savedGame.value
  if (savedIcon.status === 'fulfilled') icon.value = savedIcon.value
  if (savedWallpaper.status === 'fulfilled') wallpaper.value = savedWallpaper.value
  if (savedMusic.status === 'fulfilled') musicUrl.value = savedMusic.value
  const error = results.find((result) => result.status === 'rejected')
  if (error?.status === 'rejected') message.value = errorMessage(error.reason)
  loading.value = false
  statusTimer = setInterval(async () => {
    if (!game.value.running || busy.value) return
    try {
      game.value = await gameApi.status()
    } catch (error) {
      message.value = errorMessage(error)
    }
  }, 1600)
})

onUnmounted(() => {
  mounted = false
  document.removeEventListener('visibilitychange', syncVisibility)
  stopVisibility?.()
  if (statusTimer) clearInterval(statusTimer)
})

function openSettings() {
  settingsError.value = ''
  settings.value?.showModal()
}

async function runSetting(action: () => Promise<void>) {
  if (!desktopWindow || busy.value) return
  busy.value = true
  settingsError.value = ''
  try { await action() }
  catch (error) { settingsError.value = errorMessage(error) }
  finally { busy.value = false }
}

function chooseGame() {
  return runSetting(async () => {
    const path = await gameApi.choosePath()
    if (!path) return
    game.value = await gameApi.status()
    message.value = ''
  })
}

function changeMedia(slot: MediaSlot, reset = false) {
  return runSetting(async () => {
    const target = slot === 'icon' ? icon : wallpaper
    if (reset) {
      await mediaApi.reset(slot)
      target.value = null
    } else {
      const result = await mediaApi.choose(slot)
      if (!result) return
      target.value = result
    }
    if (slot === 'wallpaper') message.value = ''
  })
}

function importWorkshop() {
  return runSetting(async () => {
    importingWorkshop.value = true
    try {
      wallpaper.value = await mediaApi.importWorkshop(workshopItem.value)
      message.value = ''
    } finally {
      importingWorkshop.value = false
    }
  })
}

function toggleMusic() {
  if (!musicUrl.value) {
    openSettings()
    return
  }
  musicMuted.value = !musicMuted.value
}

function changeMusic(remove = false) {
  return runSetting(async () => {
    if (remove) {
      await musicApi.remove()
      musicUrl.value = null
    } else {
      const url = await musicApi.choose()
      if (!url) return
      musicUrl.value = url
    }
    musicError.value = ''
  })
}

async function launch() {
  if (!desktopWindow || busy.value || loading.value || game.value.running) return
  if (!game.value.available) { openSettings(); return }
  message.value = ''
  busy.value = true
  try {
    await gameApi.launch()
    await windowAction('close')
    game.value = await gameApi.status()
    if (!game.value.running && !message.value) message.value = '已打开所选程序。'
  } catch (error) {
    message.value = errorMessage(error)
    const status = await gameApi.status().catch(() => null)
    if (status) game.value = status
  } finally {
    busy.value = false
  }
}

async function windowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
  if (!desktopWindow) return
  const wasVisible = trayVisible.value
  const hidesWindow = action !== 'toggleMaximize'
  try {
    if (hidesWindow) {
      trayVisible.value = false
      musicElement.value?.pause()
    }
    if (action === 'close') await desktopWindow.hide()
    else await desktopWindow[action]()
  } catch (error) {
    if (hidesWindow) trayVisible.value = wasVisible
    message.value = errorMessage(error)
  }
}
</script>

<template>
  <div class="launcher">
    <header class="titlebar" data-tauri-drag-region>
      <div class="wordmark" data-tauri-drag-region>
        <img :src="iconUrl" alt="" width="23" height="23" @error="icon = null" />
        <span data-tauri-drag-region>原神启动器</span>
      </div>
      <div class="titlebar-tools">
        <button class="settings-entry music-toggle" type="button" :disabled="loading || !desktopWindow"
          :aria-label="musicUrl ? '背景音乐' : '设置背景音乐'" :aria-pressed="musicUrl ? !musicMuted : undefined"
          :title="musicButtonTitle" @click="toggleMusic">
          <VolumeX v-if="musicUrl && musicMuted" :size="17" aria-hidden="true" />
          <Music2 v-else :size="17" aria-hidden="true" />
        </button>
        <button class="settings-entry" type="button" :disabled="loading" @click="openSettings">
          <Settings2 :size="17" aria-hidden="true" />设置
        </button>
      </div>
      <div v-if="desktopWindow" class="window-actions">
        <button aria-label="最小化" title="最小化" @click="windowAction('minimize')"><Minus :size="16" /></button>
        <button aria-label="最大化或还原" title="最大化或还原" @click="windowAction('toggleMaximize')"><Square :size="13" /></button>
        <button class="window-close" aria-label="关闭到系统托盘" title="关闭到系统托盘" @click="windowAction('close')"><X :size="17" /></button>
      </div>
    </header>

    <main class="stage" :class="{ 'stage-contain': backgroundFit === 'contain' }">
      <GameBackdrop :key="wallpaper?.url ?? defaultCover" :cover-url="defaultCover"
        :wallpaper="wallpaper" :fit="backgroundFit" :active="mediaActive"
        @error="message = '无法显示所选背景，已切回默认图片。'" />
      <div class="stage-shade" aria-hidden="true"></div>
      <div class="launch-panel">
        <div class="game-identity">
          <h1>原神</h1>
          <p class="game-status" role="status"><span v-if="desktopWindow" class="status-light"
            :class="{ available: game.available }"></span>{{ statusText }}</p>
          <p v-if="message" class="feedback" role="status">{{ message }}</p>
        </div>
        <button class="play-button" :disabled="!desktopWindow || loading || busy || game.running" @click="launch">
          <Play :size="20" fill="currentColor" aria-hidden="true" />
          {{ game.running ? '运行中' : '开始游戏' }}
        </button>
      </div>
    </main>
    <audio v-if="musicUrl" :key="musicUrl" ref="musicElement" :src="musicUrl" :muted="musicMuted" loop preload="auto"
      @playing="musicError = ''" @error="musicError = '背景音乐无法播放，请重新选择音频文件。'" />

    <dialog ref="settings" aria-labelledby="settings-title" @cancel="busy && $event.preventDefault()">
      <div class="dialog-header">
        <h2 id="settings-title">启动器设置</h2>
        <button aria-label="关闭设置" :disabled="busy" @click="settings?.close()"><X :size="20" /></button>
      </div>
      <p v-if="!desktopWindow" class="notice">本地文件设置需要在 Tauri 桌面端使用。</p>
      <section class="setting-row">
        <div>
          <h3>游戏程序</h3>
          <p class="current-path" :title="game.executablePath ?? undefined">{{ game.executablePath ?? '选择原神或官方启动器的 .exe' }}</p>
        </div>
        <div class="setting-actions">
          <button class="outline-button" :disabled="!desktopWindow || busy || game.running" @click="chooseGame">选择程序</button>
        </div>
      </section>
      <section class="setting-row">
        <div><h3>软件图标</h3><p>PNG 或 ICO，应用到窗口标题栏和系统托盘</p></div>
        <div class="setting-actions">
          <button v-if="icon" class="text-button" :disabled="busy" aria-label="恢复默认图标" @click="changeMedia('icon', true)">恢复默认</button>
          <button class="outline-button" :disabled="!desktopWindow || busy" @click="changeMedia('icon')">更换图标</button>
        </div>
      </section>
      <section class="setting-row">
        <div><h3>游戏背景</h3><p>图片或 MP4、WebM 视频，视频静音循环</p></div>
        <div class="setting-actions">
          <button v-if="wallpaper" class="text-button" :disabled="busy" aria-label="恢复默认背景" @click="changeMedia('wallpaper', true)">恢复默认</button>
          <button class="outline-button" :disabled="!desktopWindow || busy" @click="changeMedia('wallpaper')">选择本地文件</button>
        </div>
      </section>
      <div class="setting-row">
        <div>
          <label class="setting-label" for="background-fit">背景适配</label>
          <p id="background-fit-hint">铺满可能裁切边缘，完整显示会用壁纸画面柔化补边。</p>
        </div>
        <select id="background-fit" v-model="backgroundFit" aria-describedby="background-fit-hint">
          <option value="cover">铺满窗口</option>
          <option value="contain">完整显示</option>
        </select>
      </div>
      <section class="setting-row">
        <div>
          <h3>背景音乐</h3>
          <p>{{ musicUrl ? (musicMuted ? '已导入 · 已静音' : '已导入 · 循环播放') : 'MP3、WAV、FLAC 或 M4A，导入后循环播放。' }}</p>
          <p v-if="musicError" class="form-error music-error" role="alert">{{ musicError }}</p>
        </div>
        <div class="setting-actions">
          <button v-if="musicUrl" class="text-button" type="button" :disabled="busy" @click="changeMusic(true)">移除音乐</button>
          <button class="outline-button" type="button" :disabled="!desktopWindow || busy" @click="changeMusic()">
            {{ musicUrl ? '更换音乐' : '导入音乐' }}
          </button>
        </div>
      </section>
      <form class="workshop-background" @submit.prevent="importWorkshop">
        <label for="workshop-item">Steam 创意工坊壁纸</label>
        <div class="workshop-field">
          <input id="workshop-item" v-model.trim="workshopItem" type="text" maxlength="512"
            placeholder="粘贴创意工坊链接或作品 ID" :disabled="!desktopWindow || busy" aria-describedby="workshop-hint" required />
          <button class="outline-button" :disabled="!desktopWindow || busy || !workshopItem">导入背景</button>
        </div>
        <p id="workshop-hint" class="notice">先在 Steam 订阅并下载。场景壁纸需打开 Wallpaper Engine，跳过前 6 秒后录制 20 秒。</p>
      </form>
      <p v-if="busy" class="notice" role="status">{{ importingWorkshop ? '正在导入壁纸，场景录制时请保持播放窗口开启。' : '正在处理…' }}</p>
      <p v-if="settingsError" class="form-error" role="alert">{{ settingsError }}</p>
    </dialog>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import { mediaApi, type GameWallpaper, type VideoCrop } from '../api/customization'

const props = defineProps<{
  coverUrl?: string | null
  wallpaper?: GameWallpaper | null
  fit: 'cover' | 'contain'
  active: boolean
}>()
const emit = defineEmits<{ error: [] }>()
const video = ref<HTMLVideoElement>()
const fillVideo = ref<HTMLVideoElement>()
const crop = ref<VideoCrop | null>(null)
const size = ref({ width: 0, height: 0 })
const ready = ref(false)
const failed = ref(false)

watch([video, fillVideo, () => props.active], ([main, fill, active]) => {
  for (const element of [main, fill]) {
    if (!element) continue
    if (active) {
      void element.play().catch(() => {
        if (element === main && element.error) fail()
      })
    } else {
      element.pause()
    }
  }
}, { flush: 'post' })

watch(() => props.wallpaper, async (wallpaper, _, onCleanup) => {
  crop.value = null
  if (!isTauri() || wallpaper?.kind !== 'video') return
  let current = true
  onCleanup(() => { current = false })
  const bounds = await mediaApi.videoCrop().catch(() => null)
  if (current) crop.value = bounds
}, { immediate: true })

const cropLayout = computed(() => {
  const { width, height } = size.value
  const bounds = crop.value
  if (!bounds || !width || !height || bounds.width <= 0 || bounds.height <= 0) return null
  const { x, y, width: cropWidth, height: cropHeight } = bounds
  // Restrict trimming to symmetric margins, including a small codec rounding tolerance.
  if (x < 0 || y < 0 || x + cropWidth > width || y + cropHeight > height
    || Math.abs(width - cropWidth - x * 2) > 4 || Math.abs(height - cropHeight - y * 2) > 4) return null
  // Keep interpolation inside the picture where the compressed border touches it.
  const insetX = x > 0 ? 2 : 0
  const insetY = y > 0 ? 2 : 0
  const visibleWidth = cropWidth - insetX * 2
  const visibleHeight = cropHeight - insetY * 2
  if (visibleWidth <= 0 || visibleHeight <= 0) return null
  return {
    frame: { '--crop-aspect': String(visibleWidth / visibleHeight) },
    media: {
      width: `${100 * width / visibleWidth}%`,
      height: `${100 * height / visibleHeight}%`,
      left: `${-100 * (x + insetX) / visibleWidth}%`,
      top: `${-100 * (y + insetY) / visibleHeight}%`,
    },
  }
})

function updateVideoSize() {
  if (video.value) size.value = { width: video.value.videoWidth, height: video.value.videoHeight }
}

function syncFill() {
  const main = video.value
  const fill = fillVideo.value
  if (main && fill && fill.readyState >= 1 && Math.abs(main.currentTime - fill.currentTime) > 0.35) {
    fill.currentTime = main.currentTime
  }
}

function fail() {
  if (failed.value) return
  failed.value = true
  emit('error')
}
</script>

<template>
  <img v-if="coverUrl && fit === 'contain'" class="stage-cover wallpaper-fill" :src="coverUrl" alt="" />
  <img v-if="coverUrl" class="stage-cover" :src="coverUrl" alt="" />
  <div v-if="wallpaper && !failed" class="wallpaper-layer" :class="{ ready }" aria-hidden="true">
    <div v-if="fit === 'contain'" class="wallpaper-fill-frame">
      <div class="wallpaper-crop" :class="{ cropped: cropLayout }" :style="cropLayout?.frame">
        <video v-if="wallpaper.kind === 'video'" ref="fillVideo" class="stage-cover"
          :style="cropLayout?.media" :src="wallpaper.url" :autoplay="active" muted loop playsinline
          @loadedmetadata="syncFill" />
        <img v-else class="stage-cover" :src="wallpaper.url" alt="" />
      </div>
    </div>
    <div class="wallpaper-crop wallpaper-foreground" :class="{ cropped: cropLayout }" :style="cropLayout?.frame">
      <video v-if="wallpaper.kind === 'video'" ref="video" class="stage-cover custom-wallpaper"
        :style="cropLayout?.media" :src="wallpaper.url" :autoplay="active" muted loop playsinline
        @loadedmetadata="updateVideoSize" @loadeddata="ready = true" @timeupdate="syncFill" @error="fail" />
      <img v-else class="stage-cover custom-wallpaper" :src="wallpaper.url" alt=""
        @load="ready = true" @error="fail" />
    </div>
  </div>
</template>

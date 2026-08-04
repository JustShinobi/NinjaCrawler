import { convertFileSrc } from '@tauri-apps/api/core'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  applyLightboxMediaPrefs,
  getStoredLightboxMediaPrefs,
  readPrefsFromElement,
  setStoredLightboxMediaPrefs,
  type LightboxMediaPrefs,
} from './lightboxSession'

const VIDEO_HYDRATE_DEBOUNCE_MS = 150

export interface MediaViewerProps {
  fileAbsPath: string
  isVideo: boolean
  audioAbsPath?: string
  autoPlay?: boolean
  loop?: boolean
  controls?: boolean
  className?: string
  alt?: string
  onVideoElement?: (element: HTMLVideoElement | null) => void
  onAudioElement?: (element: HTMLAudioElement | null) => void
}

/**
 * Shared media renderer used by lightboxes and embedded comparison previews.
 * It owns delayed video hydration and the persisted volume/mute contract so a
 * new surface cannot accidentally drift from Profile View behavior.
 */
export function MediaViewer({
  fileAbsPath,
  isVideo,
  audioAbsPath,
  autoPlay = false,
  loop = false,
  controls = true,
  className,
  alt = '',
  onVideoElement,
  onAudioElement,
}: MediaViewerProps) {
  const [initialPrefs] = useState<LightboxMediaPrefs>(getStoredLightboxMediaPrefs)
  const prefsRef = useRef(initialPrefs)
  const [hydratedVideoPath, setHydratedVideoPath] = useState<string | undefined>(() =>
    isVideo ? fileAbsPath : undefined,
  )

  useEffect(() => {
    if (!isVideo || hydratedVideoPath === fileAbsPath) return
    const timer = window.setTimeout(
      () => setHydratedVideoPath(fileAbsPath),
      VIDEO_HYDRATE_DEBOUNCE_MS,
    )
    return () => window.clearTimeout(timer)
  }, [fileAbsPath, hydratedVideoPath, isVideo])

  const setVideoElement = useCallback((element: HTMLVideoElement | null) => {
    if (element) applyLightboxMediaPrefs(element, prefsRef.current)
    onVideoElement?.(element)
  }, [onVideoElement])

  const setAudioElement = useCallback((element: HTMLAudioElement | null) => {
    if (element) applyLightboxMediaPrefs(element, prefsRef.current)
    onAudioElement?.(element)
  }, [onAudioElement])

  const rememberPrefs = useCallback((element: HTMLMediaElement) => {
    const prefs = readPrefsFromElement(element)
    prefsRef.current = prefs
    setStoredLightboxMediaPrefs(prefs)
  }, [])

  return (
    <div className={className} data-media-viewer="">
      {isVideo ? (
        hydratedVideoPath === fileAbsPath ? (
          <video
            ref={setVideoElement}
            src={convertFileSrc(fileAbsPath)}
            controls={controls}
            autoPlay={autoPlay}
            loop={loop}
            onVolumeChange={(event) => rememberPrefs(event.currentTarget)}
          />
        ) : null
      ) : (
        <img src={convertFileSrc(fileAbsPath)} alt={alt} loading="lazy" />
      )}
      {!isVideo && audioAbsPath ? (
        <audio
          key={audioAbsPath}
          ref={setAudioElement}
          src={convertFileSrc(audioAbsPath)}
          controls={controls}
          autoPlay={autoPlay}
          loop={loop}
          onVolumeChange={(event) => rememberPrefs(event.currentTarget)}
        />
      ) : null}
    </div>
  )
}

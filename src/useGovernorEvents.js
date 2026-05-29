import { useEffect, useRef } from 'react'

import { BASE_URL } from './api.js'

/** Heuristic: backend may send various shapes for “RAM is high”. */
export function isGovernorRamHigh(data) {
  if (data == null) return false
  if (typeof data === 'string') {
    try {
      return isGovernorRamHigh(JSON.parse(data))
    } catch {
      return false
    }
  }
  if (typeof data !== 'object') return false
  if (data.type === 'ram_high' || data.event === 'ram_high') return true
  if (data.ram_high === true || data.ramHigh === true) return true
  const p =
    data.ram_percent ??
    data.ramPercent ??
    data.memory_percent ??
    data.memoryPercent
  if (typeof p === 'number') {
    if (p > 1 && p >= 85) return true
    if (p <= 1 && p >= 0.85) return true
  }
  return false
}

/**
 * Subscribes to governor SSE. Invokes `onMessage` with parsed JSON or raw string.
 * @param {(data: unknown) => void} onMessage
 */
export function useGovernorEvents(onMessage) {
  const onMessageRef = useRef(onMessage)

  useEffect(() => {
    onMessageRef.current = onMessage
  }, [onMessage])

  useEffect(() => {
    const url = `${BASE_URL}/api/events`
    const es = new EventSource(url)

    es.onmessage = (event) => {
      let payload = event.data
      try {
        payload = JSON.parse(event.data)
      } catch {
        // keep string
      }
      onMessageRef.current?.(payload)
    }

    return () => {
      es.close()
    }
  }, [])
}

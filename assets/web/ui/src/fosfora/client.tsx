import { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import type { AudioState, FullState } from './types'

type Outbound = Record<string, unknown>

type FosforaClient = {
  connected: boolean
  linkError: string | null
  state: FullState | null
  audio: AudioState | null
  send: (message: Outbound) => void
}

const FosforaContext = createContext<FosforaClient | null>(null)

export function FosforaProvider({ children }: { children: ReactNode }) {
  const socket = useRef<WebSocket | null>(null)
  const [connected, setConnected] = useState(false)
  const [linkError, setLinkError] = useState<string | null>(null)
  const [state, setState] = useState<FullState | null>(null)
  const [audio, setAudio] = useState<AudioState | null>(null)

  useEffect(() => {
    let stopped = false
    let delay = 500
    let timer = 0

    const open = () => {
      if (stopped) return
      const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
      const ws = new WebSocket(`${protocol}//${location.host}/ws`)
      socket.current = ws
      ws.onopen = () => {
        delay = 500
        setConnected(true)
        setLinkError(null)
      }
      ws.onclose = () => {
        setConnected(false)
        if (!stopped) timer = window.setTimeout(open, delay)
        delay = Math.min(delay * 2, 4000)
      }
      ws.onmessage = (event) => {
        let message: { type?: string }
        try {
          message = JSON.parse(String(event.data)) as { type?: string }
        } catch {
          return
        }
        if (message.type === 'state') setState(message as FullState)
        if (message.type === 'audio') setAudio(message as AudioState)
      }
    }

    open()
    return () => {
      stopped = true
      window.clearTimeout(timer)
      socket.current?.close()
    }
  }, [])

  const client = useMemo<FosforaClient>(
    () => ({
      connected,
      linkError,
      state,
      audio,
      send: (message) => {
        const ws = socket.current
        if (!ws || ws.readyState !== WebSocket.OPEN) {
          setLinkError('Not connected. The command was not sent.')
          return
        }
        try {
          ws.send(JSON.stringify(message))
          setLinkError(null)
        } catch {
          setLinkError('The command could not be sent.')
        }
      },
    }),
    [audio, connected, linkError, state],
  )

  return <FosforaContext.Provider value={client}>{children}</FosforaContext.Provider>
}

export function useFosfora() {
  const client = useContext(FosforaContext)
  if (!client) throw new Error('useFosfora must be used inside FosforaProvider')
  return client
}

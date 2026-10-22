import { useMemo, useRef, useState } from 'react'
import type { DragEvent, PointerEvent, ReactNode } from 'react'
import { GripVertical, Info, Trash2 } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { HeldSlider } from '@/held-slider'
import { useFosfora } from '@/fosfora/client'
import { BLENDS, clock } from '@/fosfora/types'
import type { LayerState, LibraryItem } from '@/fosfora/types'

function activeLayer(layers: LayerState[], index: number) {
  return layers[index]
}

const MEDIA_MIME = 'application/x-fosfora-media'
const MEDIA_PREFIX = 'fosfora-media:'

export function SectionInfo({ label, text }: { label: string; text: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          className="inline-flex size-5 shrink-0 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <Info className="size-3.5" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom" className="max-w-72 text-left whitespace-normal">
        {text}
      </TooltipContent>
    </Tooltip>
  )
}

function mayBeMedia(event: DragEvent) {
  const types = [...event.dataTransfer.types]
  if (types.length === 0) return true
  return types.includes(MEDIA_MIME) || types.includes('text/plain') || types.includes('Text') ||
    types.includes('Files') || types.includes('text/uri-list')
}

function readMediaDrag(event: DragEvent) {
  const custom = event.dataTransfer.getData(MEDIA_MIME)
  if (custom) return custom
  const text = event.dataTransfer.getData('text/plain') || event.dataTransfer.getData('Text')
  return text.startsWith(MEDIA_PREFIX) ? text.slice(MEDIA_PREFIX.length) : ''
}

function fileUris(event: DragEvent) {
  return event.dataTransfer
    .getData('text/uri-list')
    .split(/\s+/)
    .map((line) => {
      const trimmed = line.trim()
      if (!trimmed.startsWith('file:')) return ''
      const path = trimmed.replace(/^file:\/\/(localhost)?/, '')
      try {
        return decodeURIComponent(path)
      } catch {
        return path
      }
    })
    .filter((path) => path.startsWith('/'))
}

function isFinderDrop(event: DragEvent) {
  const types = [...event.dataTransfer.types]
  if (types.includes(MEDIA_MIME)) return false
  return types.includes('Files') || types.includes('text/uri-list')
}

function writeMediaDrag(event: DragEvent, id: string) {
  event.dataTransfer.setData(MEDIA_MIME, id)
  event.dataTransfer.setData('text/plain', `${MEDIA_PREFIX}${id}`)
  event.dataTransfer.effectAllowed = 'copy'
}

type LayerDrag = {
  pointerId: number
  from: number
  to: number
  startY: number
  offset: number
  height: number
  mids: number[]
}

function neighborShift(index: number, drag: LayerDrag | null) {
  if (!drag || index === drag.from) return 0
  if (drag.from < drag.to && index > drag.from && index <= drag.to) return -1
  if (drag.from > drag.to && index < drag.from && index >= drag.to) return 1
  return 0
}

export function LayersPanel() {
  const { state, send } = useFosfora()
  const layers = state?.layers ?? []
  const cards = useRef<Array<HTMLDivElement | null>>([])
  const dragRef = useRef<LayerDrag | null>(null)
  const [drag, setDrag] = useState<LayerDrag | null>(null)
  const [dropAt, setDropAt] = useState<number | 'new' | null>(null)
  const [mediaNotice, setMediaNotice] = useState<string | null>(null)
  const gap = 12

  function beginDrag(event: PointerEvent<HTMLButtonElement>, index: number) {
    if (layers[index]?.pinned) return
    const mids = cards.current.map((card) => {
      if (!card) return 0
      const rect = card.getBoundingClientRect()
      return rect.top + rect.height / 2
    })
    event.preventDefault()
    try {
      event.currentTarget.setPointerCapture(event.pointerId)
    } catch {
      // The handle still receives the gesture when the pointer stays on it.
    }
    const next = {
      pointerId: event.pointerId,
      from: index,
      to: index,
      startY: event.clientY,
      offset: 0,
      height: cards.current[index]?.offsetHeight ?? 72,
      mids,
    }
    dragRef.current = next
    setDrag(next)
  }

  function moveDrag(event: PointerEvent<HTMLButtonElement>) {
    const current = dragRef.current
    if (!current || current.pointerId !== event.pointerId) return
    let to = current.from
    let best = Number.POSITIVE_INFINITY
    current.mids.forEach((mid, slot) => {
      const distance = Math.abs(event.clientY - mid)
      if (distance < best) {
        best = distance
        to = slot
      }
    })
    const next = { ...current, to, offset: event.clientY - current.startY }
    dragRef.current = next
    setDrag(next)
  }

  function endDrag(event: PointerEvent<HTMLButtonElement>) {
    const current = dragRef.current
    if (!current || current.pointerId !== event.pointerId) return
    if (current.to !== current.from) send({ type: 'move_layer', from: current.from, to: current.to })
    dragRef.current = null
    setDrag(null)
  }

  function takeMedia(event: DragEvent, index: number | 'new') {
    if (!mayBeMedia(event)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
    setDropAt((current) => (current === index ? current : index))
  }

  function leaveMedia(event: DragEvent<HTMLElement>) {
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
    setDropAt(null)
  }

  async function uploadFinderFile(file: File) {
    const limit = 512 * 1024 * 1024
    if (file.size > limit) {
      setMediaNotice(file.name + ' is over the 512 MB upload limit. Nothing was added.')
      return
    }
    setMediaNotice('Uploading ' + file.name + ' to the show library…')
    try {
      const body = new FormData()
      body.append('file', file, file.name)
      const response = await fetch('/api/media', { method: 'POST', body })
      if (!response.ok) {
        const reason = await response.text()
        throw new Error(reason || 'HTTP ' + response.status)
      }
      const result = (await response.json()) as {
        files?: Array<{ id?: string; ok?: boolean }>
      }
      const id = result.files?.find((item) => item.ok && item.id)?.id
      if (!id) throw new Error('The server did not confirm a stored media file')
      // Uploads are already committed to the configured external media
      // folder, but the background Library index may not yet include them.
      // add_media accepts the portable ID and resolves it within the pack.
      send({ type: 'add_media', path: id })
      send({ type: 'rescan_library' })
      setMediaNotice(file.name + ' stored. Fosfora is decoding it; the new layer appears when ready. If decoding fails, check the Show notice.')
    } catch (error) {
      setMediaNotice(file.name + ' was not added: ' + (error instanceof Error ? error.message : String(error)))
    }
  }

  function dropMedia(event: DragEvent, index: number | 'new') {
    setDropAt(null)
    const id = readMediaDrag(event)
    if (id) {
      event.preventDefault()
      event.stopPropagation()
      setMediaNotice(null)
      if (index === 'new') send({ type: 'add_library_media', ids: [id] })
      else send({ type: 'replace_library_media', layer: index, id })
      return
    }

    if (!isFinderDrop(event)) return
    event.preventDefault()
    event.stopPropagation()
    const files = [...event.dataTransfer.files]
    if (index !== 'new') {
      setMediaNotice('To replace a layer, first add the Finder file to Library, then drag the Library item onto that layer.')
    } else if (files.length !== 1) {
      setMediaNotice(files.length === 0
        ? 'No readable Finder file was provided. Drop the file into Library first.'
        : 'Drop one Finder file at a time to create a layer.')
    } else {
      void uploadFinderFile(files[0])
    }
  }

  return (
    <div className={`flex h-full min-h-0 flex-1 flex-col gap-2 ${drag ? 'select-none' : ''}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5">
          <span className="text-sm font-medium">Layers</span>
          <SectionInfo
            label="About layers"
            text="Click a layer to choose what Look and Picture edit. That click does not change the picture. Opacity, blend, and enable do. Drag the grip to reorder; the top of the list is in front. Drop a Library item on a layer to replace it, or on the dashed row to add. A Finder file dropped on the dashed row is uploaded and then decoded; it may take time to appear. Solo shows only one layer and is not saved."
          />
        </div>
        <Button
          variant={state?.solo_layer != null ? 'default' : 'outline'}
          onClick={() => {
            if (state?.solo_layer != null) send({ type: 'clear_solo' })
            else send({ type: 'set_solo', index: state?.active_layer ?? 0 })
          }}
        >
          {state?.solo_layer != null ? 'Exit solo' : 'Solo selected'}
        </Button>
      </div>
      <div
        className={`rounded-xl border border-dashed px-3 py-2 text-center text-xs ${dropAt === 'new' ? 'border-primary bg-primary/15 text-foreground' : 'border-border text-muted-foreground'}`}
        onDragEnter={(event) => takeMedia(event, 'new')}
        onDragOver={(event) => takeMedia(event, 'new')}
        onDragLeave={leaveMedia}
        onDrop={(event) => dropMedia(event, 'new')}
      >
        Drop a Library or Finder file here to add a layer
      </div>
      {mediaNotice ? <p role="status" className="text-xs text-muted-foreground">{mediaNotice}</p> : null}
      <div className="min-h-0 flex-1 space-y-2 overflow-auto pr-1">
      {layers.map((layer, index) => {
        const selected = index === state?.active_layer
        const solo = state?.solo_layer === index
        const shift = neighborShift(index, drag)
        const dragging = drag?.from === index
        const travel = dragging ? drag.offset : shift * (drag ? drag.height + gap : 0)
        const removeLocked = layer.pinned || layers.length < 2
        return (
          <ContextMenu
            key={`${layer.effect_name || layer.media_name || layer.name}-${index}`}
            onOpenChange={(open) => {
              if (open) send({ type: 'select_layer', index })
            }}
          >
          <ContextMenuTrigger asChild>
          <Card
            size="sm"
            ref={(node) => {
              cards.current[index] = node
            }}
            className={`${selected ? 'bg-primary/20 shadow-[inset_3px_0_0_0_var(--primary)] ring-primary/40' : ''} ${dropAt === index ? 'bg-primary/25 ring-2 ring-primary' : ''} ${dragging ? 'z-20 shadow-lg' : ''}`}
            style={{
              transform: travel ? `translateY(${travel}px)` : undefined,
              transition: dragging ? 'none' : 'transform 180ms cubic-bezier(.2,.7,.2,1)',
              position: 'relative',
            }}
            onClick={() => send({ type: 'select_layer', index })}
            onDragEnter={(event) => takeMedia(event, index)}
            onDragOver={(event) => takeMedia(event, index)}
            onDragLeave={leaveMedia}
            onDrop={(event) => dropMedia(event, index)}
          >
            <CardContent className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  className="cursor-grab touch-none text-muted-foreground active:cursor-grabbing disabled:cursor-not-allowed"
                  disabled={layer.pinned}
                  aria-label={`Drag ${layer.name}`}
                  onClick={(event) => event.stopPropagation()}
                  onPointerDown={(event) => beginDrag(event, index)}
                  onPointerMove={moveDrag}
                  onPointerUp={endDrag}
                  onPointerCancel={endDrag}
                >
                  <GripVertical className="size-4" />
                </button>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate font-medium">{layer.name}</span>
                    {layer.effect_name ? <Badge variant="secondary">{layer.effect_name}</Badge> : null}
                    {solo ? <Badge>Solo</Badge> : null}
                    {layer.pinned ? <Badge variant="outline">Pinned</Badge> : null}
                    {dropAt === index ? <span className="text-xs text-muted-foreground">Replace</span> : null}
                  </div>
                </div>
                <button
                  type="button"
                  aria-label={`Remove ${layer.name}`}
                  title={layer.pinned ? 'Pinned layers stay' : layers.length < 2 ? 'The last layer stays' : `Remove ${layer.name}`}
                  disabled={layer.pinned || layers.length < 2}
                  className="text-muted-foreground hover:text-destructive disabled:pointer-events-none disabled:opacity-30"
                  onClick={(event) => {
                    event.stopPropagation()
                    send({ type: 'remove_layer', index })
                  }}
                >
                  <Trash2 className="size-4" />
                </button>
                <Switch
                  checked={layer.enabled}
                  disabled={layer.locked}
                  onClick={(event) => event.stopPropagation()}
                  onCheckedChange={(checked) =>
                    send({ type: 'set_layer_enabled', layer: index, value: checked })
                  }
                />
              </div>
              <div className="flex items-center gap-2" onClick={(event) => event.stopPropagation()}>
                <span className="w-10 text-xs text-muted-foreground">
                  {Math.round(layer.opacity * 100)}%
                </span>
                <HeldSlider
                  value={Math.round(layer.opacity * 100)}
                  disabled={layer.locked}
                  onChange={(value) =>
                    send({ type: 'set_layer_opacity', layer: index, value: value / 100 })
                  }
                />
                <Select
                  value={String(layer.blend_mode)}
                  onValueChange={(value) =>
                    send({ type: 'set_layer_blend', layer: index, value: Number(value) })
                  }
                >
                  <SelectTrigger className="w-32">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent position="popper" sideOffset={4} className="max-h-72">
                    {BLENDS.map((name, blend) => (
                      <SelectItem key={name} value={String(blend)}>
                        {name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </CardContent>
          </Card>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem
              onSelect={() => {
                if (solo) send({ type: 'clear_solo' })
                else send({ type: 'set_solo', index })
              }}
            >
              {solo ? 'Exit solo' : 'Solo'}
            </ContextMenuItem>
            <ContextMenuItem
              disabled={layer.locked}
              onSelect={() => send({ type: 'set_layer_enabled', layer: index, value: !layer.enabled })}
            >
              {layer.enabled ? 'Disable' : 'Enable'}
            </ContextMenuItem>
            <ContextMenuSub>
              <ContextMenuSubTrigger>Blend</ContextMenuSubTrigger>
              <ContextMenuSubContent>
                {BLENDS.map((name, blend) => (
                  <ContextMenuItem
                    key={name}
                    onSelect={() => send({ type: 'set_layer_blend', layer: index, value: blend })}
                  >
                    <span className="flex-1">{name}</span>
                    {layer.blend_mode === blend ? <span className="text-xs text-muted-foreground">On</span> : null}
                  </ContextMenuItem>
                ))}
              </ContextMenuSubContent>
            </ContextMenuSub>
            <ContextMenuSeparator />
            <ContextMenuItem
              variant="destructive"
              disabled={removeLocked}
              onSelect={() => send({ type: 'remove_layer', index })}
            >
              Remove
            </ContextMenuItem>
          </ContextMenuContent>
          </ContextMenu>
        )
      })}
      </div>
    </div>
  )
}

export function LookPanel() {
  const { state, send } = useFosfora()
  const layer = state ? activeLayer(state.layers, state.active_layer) : undefined
  const [query, setQuery] = useState('')
  const effects = (state?.effects ?? []).filter((effect) =>
    effect.name.toLowerCase().includes(query.trim().toLowerCase()),
  )

  return (
    <div className="grid gap-4 lg:grid-cols-[280px_1fr]">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            Effects
            <SectionInfo
              label="About effects"
              text="Loads onto the layer you selected on Layers. The highlighted name is the effect already on that layer. Choosing one replaces the look on that layer and leaves the other layers as they are."
            />
          </CardTitle>
          <CardDescription>Loads onto {layer?.name ?? 'the selected layer'}.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Input value={query} placeholder="Search effects" onChange={(event) => setQuery(event.target.value)} />
          <ScrollArea className="h-[28rem]">
            <div className="flex flex-col gap-1 pr-3">
              {effects.map((effect) => (
                <Button
                  key={effect.index}
                  variant={layer?.effect_name === effect.name ? 'secondary' : 'ghost'}
                  className="justify-start"
                  onClick={() => send({ type: 'load_effect', index: effect.index })}
                >
                  {effect.name}
                </Button>
              ))}
            </div>
          </ScrollArea>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            {layer?.effect_name || layer?.name || 'Parameters'}
            <SectionInfo
              label="About parameters"
              text="These controls belong to the selected layer. A badge means audio or the palette is driving the picture. The slider then writes the saved value, which comes back when that control stops."
            />
          </CardTitle>
          <CardDescription>
            Audio-bound parameters follow the music. The saved scene keeps the value you set.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {(layer?.params ?? [])
            .filter((param) => param.type === 'float' || param.type === 'bool')
            .map((param) => {
              const driven = param.control === 'audio' || param.control === 'palette'
              const savedNorm = param.authored ?? param.value
              if (param.type === 'bool') {
                return (
                  <label key={param.name} className="flex items-center justify-between gap-3 text-sm">
                    <span className="flex items-center gap-2">
                      {param.name}
                      {driven ? <Badge variant="outline">{param.control}</Badge> : null}
                    </span>
                    <Switch
                      checked={(driven ? savedNorm : param.value) > 0.5}
                      onCheckedChange={(checked) =>
                        send({
                          type: 'set_layer_param',
                          layer: state?.active_layer ?? 0,
                          name: param.name,
                          value: checked ? 1 : 0,
                        })
                      }
                    />
                  </label>
                )
              }
              const min = param.min ?? 0
              const max = param.max ?? 1
              const live = min + (max - min) * param.value
              const saved = min + (max - min) * savedNorm
              return (
                <div key={param.name} className="space-y-2">
                  <div className="flex justify-between gap-3 text-sm">
                    <span className="flex items-center gap-2">
                      {param.name}
                      {driven ? <Badge variant="outline">{param.control}</Badge> : null}
                    </span>
                    <span className="text-muted-foreground">
                      {driven ? `live ${live.toFixed(2)} · saved ${saved.toFixed(2)}` : live.toFixed(2)}
                    </span>
                  </div>
                  <HeldSlider
                    max={1000}
                    value={Math.round((driven ? savedNorm : param.value) * 1000)}
                    onChange={(value) =>
                      send({
                        type: 'set_layer_param',
                        layer: state?.active_layer ?? 0,
                        name: param.name,
                        value: value / 1000,
                      })
                    }
                  />
                  {driven ? (
                    <p className="text-xs text-muted-foreground">
                      The picture follows {param.control}. This slider writes the saved value, which returns when that control stops.
                    </p>
                  ) : null}
                </div>
              )
            })}
        </CardContent>
      </Card>
    </div>
  )
}

export function PicturePanel() {
  const { state, send } = useFosfora()
  const layer = state ? activeLayer(state.layers, state.active_layer) : undefined
  const media = layer?.media_name ? layer : undefined

  function place(fit: string, width: number, x: number, y: number) {
    if (!media) return
    send({
      type: 'set_media_place',
      layer: media.index,
      fit,
      width,
      x,
      y,
    })
  }

  if (!media) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            Picture
            <SectionInfo
              label="About picture"
              text="Place, scale, and play the video or image on the selected layer. Fill covers the frame, fit shows the whole picture, and width sets the size. Select a video or image layer first."
            />
          </CardTitle>
          <CardDescription>Select a video or image layer to place it.</CardDescription>
        </CardHeader>
      </Card>
    )
  }

  const fit = media.media_fit || 'fit'
  const width = media.media_width ?? 1
  const x = media.media_pos_x ?? 0
  const y = media.media_pos_y ?? 0

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-1.5">
          {media.name}
          <SectionInfo
            label="About picture"
            text="Place, scale, and play this layer's video or image. Fill covers the frame, fit shows the whole picture, and width sets the size. Speed and loop apply to video."
          />
        </CardTitle>
        <CardDescription>{media.media_name}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="flex flex-wrap gap-2">
          {(['fill', 'fit', 'width'] as const).map((mode) => (
            <Button
              key={mode}
              variant={fit === mode ? 'default' : 'outline'}
              onClick={() => place(mode, width, x, y)}
            >
              {mode}
            </Button>
          ))}
          <Button variant="outline" onClick={() => place(fit, 1, 0, 0)}>
            Reset
          </Button>
        </div>
        <Field label="Width" value={`${Math.round(width * 100)}%`}>
          <HeldSlider
            min={25}
            max={300}
            value={Math.round(width * 100)}
            onChange={(value) => place('width', value / 100, x, y)}
          />
        </Field>
        <Field label="X" value={String(Math.round(x * 100))}>
          <HeldSlider min={-100} max={100} value={Math.round(x * 100)} onChange={(value) => place(fit, width, value / 100, y)} />
        </Field>
        <Field label="Y" value={String(Math.round(y * 100))}>
          <HeldSlider min={-100} max={100} value={Math.round(y * 100)} onChange={(value) => place(fit, width, x, value / 100)} />
        </Field>
        {media.media_is_video ? (
          <>
            <Field label="Speed" value={(media.media_speed ?? 1).toFixed(2)}>
              <HeldSlider
                min={5}
                max={400}
                value={Math.round((media.media_speed ?? 1) * 100)}
                onChange={(value) =>
                  send({ type: 'set_media_transport', layer: media.index, speed: value / 100 })
                }
              />
            </Field>
            <Button
              variant="outline"
              onClick={() =>
                send({
                  type: 'set_media_transport',
                  layer: media.index,
                  looping: media.media_looping === false,
                })
              }
            >
              {media.media_looping === false ? 'Loop off' : 'Loop on'}
            </Button>
          </>
        ) : null}
      </CardContent>
    </Card>
  )
}

function libraryMeta(item: LibraryItem) {
  const parts: string[] = []
  if (item.width && item.height) parts.push(`${item.width}×${item.height}`)
  if (item.kind === 'video' && item.fps) parts.push(`${Math.round(item.fps)} fps`)
  if (item.duration_secs && item.duration_secs > 0) parts.push(formatDuration(item.duration_secs))
  if (item.bytes) parts.push(formatBytes(item.bytes))
  return parts.join(' · ') || item.kind || 'file'
}

function formatDuration(secs: number) {
  const total = Math.round(secs)
  const minutes = Math.floor(total / 60)
  const seconds = total % 60
  return minutes > 0 ? `${minutes}:${String(seconds).padStart(2, '0')}` : `${secs.toFixed(1)}s`
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function Field({ label, value, children }: { label: string; value: string; children: ReactNode }) {
  return (
    <div className="space-y-2">
      <div className="flex justify-between text-sm">
        <span>{label}</span>
        <span className="text-muted-foreground">{value}</span>
      </div>
      {children}
    </div>
  )
}

export function LibraryPanel() {
  const { state, send } = useFosfora()
  const [query, setQuery] = useState('')
  const [picked, setPicked] = useState<string[]>([])
  const [note, setNote] = useState('')
  const [finderOver, setFinderOver] = useState(false)
  const finderPaths = useRef<Promise<string[]> | null>(null)
  const items = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return (state?.library ?? [])
      .filter((item) => !needle || item.name.toLowerCase().includes(needle))
      .slice()
      .sort((a, b) => a.name.localeCompare(b.name))
  }, [query, state?.library])

  async function upload(files: FileList | null) {
    if (!files?.length) return
    // Two files at a time. The engine refuses a third with 503 so a long
    // import never stalls the controls.
    const queue = [...files]
    let stored = 0
    async function sendOne(file: File) {
      for (let attempt = 0; attempt < 20; attempt++) {
        const body = new FormData()
        body.append('file', file, file.name)
        const response = await fetch('/api/media', { method: 'POST', body })
        if (response.status !== 503) {
          if (response.ok) stored += 1
          setNote(response.ok ? `${file.name} stored (${stored} of ${files!.length})` : `${file.name} failed: ${await response.text()}`)
          return
        }
        await new Promise((resolve) => setTimeout(resolve, 500))
      }
      setNote(`${file.name} was not sent: the engine stayed busy`)
    }
    async function worker() {
      for (let file = queue.shift(); file; file = queue.shift()) await sendOne(file)
    }
    await Promise.all([worker(), worker()])
    send({ type: 'rescan_library' })
  }

  function armFinder(event: DragEvent<HTMLElement>) {
    if (!isFinderDrop(event)) return
    event.preventDefault()
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
    finderPaths.current = fetch('/api/media/finder-drag')
      .then((response) => (response.ok ? response.json() : []))
      .then((paths) => (Array.isArray(paths) ? paths.filter((path) => typeof path === 'string') : []))
      .catch(() => [])
    setFinderOver(true)
  }

  function moveFinder(event: DragEvent) {
    if (!isFinderDrop(event)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'link'
    setFinderOver(true)
  }

  function leaveFinder(event: DragEvent<HTMLElement>) {
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
    setFinderOver(false)
  }

  async function dropFinder(event: DragEvent) {
    if (!isFinderDrop(event)) return
    event.preventDefault()
    setFinderOver(false)
    const pending = finderPaths.current
    finderPaths.current = null
    const fromPasteboard = pending ? await pending : []
    const paths = [...new Set([...fileUris(event), ...fromPasteboard])]
    if (paths.length === 0) {
      setNote('The Finder location was not available, so nothing was added and nothing was copied.')
      return
    }
    send({ type: 'link_library_files', paths })
    setNote('Registering the original file. It is not copied.')
  }

  return (
    <Card
      className={finderOver ? 'ring-2 ring-primary' : undefined}
      onDragEnter={armFinder}
      onDragOver={moveFinder}
      onDragLeave={leaveFinder}
      onDrop={(event) => void dropFinder(event)}
    >
      <CardHeader>
        <CardTitle className="flex items-center gap-1.5">
          Library
          <SectionInfo
            label="About the library"
            text="Drag a file here from Finder to register it where it already is. Nothing is copied. Drop a library item on the Layers page to replace a layer, or on the dashed row to add one. The trash icon removes it from this list. A file a scene still uses is kept. Upload is separate: that button does copy the file into the show."
          />
        </CardTitle>
        <CardDescription>
          Drop a Finder file to register its original path. Upload copies into the show.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap gap-2">
          <Input className="max-w-xs" value={query} placeholder="Search media" onChange={(event) => setQuery(event.target.value)} />
          <Button variant="outline" asChild>
            <label>
              Upload
              <input
                className="hidden"
                type="file"
                multiple
                accept=".mp4,.mov,.avi,.mkv,.webm,.m4v,.flv,.png,.jpg,.jpeg,.gif,.webp"
                onChange={(event) => {
                  void upload(event.target.files)
                  event.target.value = ''
                }}
              />
            </label>
          </Button>
          <Button
            disabled={picked.length === 0}
            onClick={() => {
              send({ type: 'add_library_media', ids: picked })
              setPicked([])
            }}
          >
            Add as layers
          </Button>
          <Button
            variant="outline"
            disabled={picked.length !== 1}
            onClick={() => send({ type: 'replace_library_media', layer: state?.active_layer ?? 0, id: picked[0] })}
          >
            Replace selected
          </Button>
        </div>
        {note ? <p className="text-sm text-muted-foreground">{note}</p> : null}
        {(state?.library_warnings ?? []).map((warning) => (
          <p key={warning} className="text-sm text-destructive">{warning}</p>
        ))}
        <ScrollArea className="h-[32rem]">
          <div className="space-y-1 pr-3">
            {items.map((item) => {
              const on = picked.includes(item.id)
              const meta = libraryMeta(item)
              return (
                <div
                  key={item.id}
                  className={`flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm hover:bg-muted ${on ? 'bg-muted' : ''}`}
                >
                  <button
                    type="button"
                    draggable
                    className="flex min-w-0 flex-1 cursor-grab items-center gap-3 text-left active:cursor-grabbing"
                    onDragStart={(event) => writeMediaDrag(event, item.id)}
                    onClick={() =>
                      setPicked((current) =>
                        current.includes(item.id) ? current.filter((id) => id !== item.id) : [...current, item.id],
                      )
                    }
                  >
                    {item.thumb ? (
                      <img
                        src={`/api/media/thumb/${encodeURIComponent(item.thumb)}`}
                        alt=""
                        draggable={false}
                        className="h-10 w-16 shrink-0 rounded-md bg-muted object-cover"
                      />
                    ) : (
                      <span className="h-10 w-16 shrink-0 rounded-md bg-muted" />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium">{item.name}</span>
                      <span className="block truncate text-xs text-muted-foreground">{meta}</span>
                      {item.location === 'linked' && item.path ? (
                        <span className="block truncate text-xs text-muted-foreground">{item.path}</span>
                      ) : null}
                      {item.status && item.status !== 'ok' ? (
                        <span className="block truncate text-xs text-destructive">
                          {item.detail || item.status}
                        </span>
                      ) : null}
                      {item.scenes && item.scenes.length > 0 ? (
                        <span className="block truncate text-xs text-muted-foreground">In {item.scenes.join(', ')}</span>
                      ) : null}
                    </span>
                    <Badge variant={item.status && item.status !== 'ok' ? 'destructive' : 'outline'}>
                      {item.location === 'linked' ? 'linked' : item.kind || 'file'}
                    </Badge>
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove ${item.name}`}
                    title={
                      item.scenes && item.scenes.length > 0
                        ? `Used by ${item.scenes.join(', ')}`
                        : item.location === 'linked'
                          ? `Remove ${item.name} from the library. The file stays where it is.`
                          : `Remove ${item.name} from the show media folder`
                    }
                    className="text-muted-foreground hover:text-destructive"
                    onClick={() => send({ type: 'remove_library_item', id: item.id })}
                  >
                    <Trash2 className="size-4" />
                  </button>
                </div>
              )
            })}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  )
}

export function ShowPanel() {
  const { state, send } = useFosfora()
  const show = state?.show
  const grade = Math.round((show?.grade_strength ?? 0.75) * 100)

  function showAction(action: string, extra?: Record<string, unknown>) {
    send({ type: 'show', action, ...extra })
  }

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            {show?.loaded ? clock(show.elapsed_ms) : 'No show'}
            <SectionInfo
              label="About the show"
              text="The show clock moves scenes and palettes on its own after Start. Prev and next step the scene or palette by hand. Blackout cuts the output. Palette auto lets the schedule choose colours again."
            />
          </CardTitle>
          <CardDescription>{show?.validation || 'Open a show pack in Fosfora.'}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex justify-between text-sm">
            <span>{show?.scene || 'Scene'}</span>
            <span className="text-muted-foreground">{show?.next_scene ? `next ${show.next_scene}` : ''}</span>
          </div>
          <div className="flex justify-between text-sm">
            <span>{show?.palette || 'Palette'}</span>
            <Badge variant="secondary">{show?.palette_manual ? 'Manual' : 'Auto'}</Badge>
          </div>
          <div className="flex flex-wrap gap-2">
            {(show?.palettes ?? []).map((id) => (
              <Button
                key={id}
                size="sm"
                variant={show?.palette === id ? 'default' : 'outline'}
                onClick={() => showAction('select_palette', { id })}
              >
                {id}
              </Button>
            ))}
          </div>
          <Field label="Palette strength" value={`${grade}%`}>
            <HeldSlider
              value={grade}
              onChange={(value) =>
                send({ type: 'set_grade', strength: value / 100, enabled: show?.grade_enabled !== false })
              }
            />
          </Field>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => showAction('prev_visual')}>Scene prev</Button>
            <Button variant="outline" onClick={() => showAction('next_visual')}>Scene next</Button>
            <Button variant="outline" onClick={() => showAction('prev_palette')}>Palette prev</Button>
            <Button variant="outline" onClick={() => showAction('next_palette')}>Palette next</Button>
            <Button variant="outline" onClick={() => showAction('start')}>Start</Button>
            <Button variant="outline" onClick={() => showAction('pause')}>Pause</Button>
            <Button variant="outline" onClick={() => showAction('resume')}>Resume</Button>
            <Button variant="outline" onClick={() => showAction('stop_auto')}>Stop auto</Button>
            <Button variant="outline" onClick={() => showAction('reset')}>Reset</Button>
            <Button variant={show?.blackout ? 'destructive' : 'outline'} onClick={() => showAction('toggle_blackout')}>
              {show?.blackout ? 'Blackout on' : 'Blackout off'}
            </Button>
            <Button variant="outline" onClick={() => showAction('palette_auto')}>Palette auto</Button>
          </div>
          {show?.notice ? <p className="text-sm text-muted-foreground">{show.notice}</p> : null}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            Presets
            <SectionInfo
              label="About presets"
              text="The highlighted name is the scene Fosfora has applied. A scene stays unmarked while it is still loading. In Perform, switching scenes waits until you save, discard, or cancel."
            />
          </CardTitle>
          <CardDescription>Loading a scene in Perform waits if the current scene is unsaved.</CardDescription>
        </CardHeader>
        <CardContent>
          <ScrollArea className="h-[28rem]">
            <div className="flex flex-col gap-1 pr-3">
              {(state?.presets ?? []).map((preset) => {
                const applied = state?.applied_preset === preset.name
                const loading = state?.loading_preset === preset.name
                const waiting =
                  state?.requested_preset === preset.name && !applied && !loading
                return (
                  <Button
                    key={preset.index}
                    variant={applied ? 'secondary' : 'ghost'}
                    className="justify-start"
                    onClick={() => send({ type: 'load_preset', index: preset.index })}
                  >
                    {preset.name}
                    {loading ? ' · loading' : ''}
                    {waiting ? ' · queued' : ''}
                    {preset.builtin ? <Badge variant="outline">Built-in</Badge> : null}
                  </Button>
                )
              })}
            </div>
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  )
}

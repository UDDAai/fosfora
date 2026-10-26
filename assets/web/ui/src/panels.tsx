import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { DragEvent, PointerEvent, ReactNode } from 'react'
import { GripVertical, Info, Pause, Play, Square, Star, Trash2 } from 'lucide-react'
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

const STARS_KEY = 'fosfora-stars'

function readStars() {
  try {
    const parsed = JSON.parse(localStorage.getItem(STARS_KEY) || '[]')
    return Array.isArray(parsed) ? parsed.filter((item) => typeof item === 'string') : []
  } catch {
    return [] as string[]
  }
}

function useStars() {
  const [stars, setStars] = useState<string[]>(readStars)
  function toggle(key: string) {
    setStars((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
      localStorage.setItem(STARS_KEY, JSON.stringify(next))
      return next
    })
  }
  return { stars, toggle }
}

function starKeyForLayer(layer: LayerState) {
  if (layer.effect_name) return `effect:${layer.effect_name}`
  if (layer.media_name) return `media:${layer.media_name}`
  return `layer:${layer.name}`
}

function StarMark({ on, label, onToggle }: { on: boolean; label: string; onToggle: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label={on ? `Unstar ${label}` : `Star ${label}`}
      title={on ? `Unstar ${label}` : `Star ${label}`}
      className={`inline-flex size-7 shrink-0 items-center justify-center rounded-md ${on ? 'text-amber-400' : 'text-muted-foreground hover:text-foreground'}`}
      onClick={(event) => {
        event.stopPropagation()
        onToggle()
      }}
    >
      <Star className={`size-4 ${on ? 'fill-current' : ''}`} />
    </button>
  )
}

function swatchCss(rgba: number[]) {
  const [r = 0, g = 0, b = 0, a = 1] = rgba
  return `rgba(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)}, ${a})`
}

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
  return types.includes(MEDIA_MIME) || types.includes('text/plain') || types.includes('Text')
}

function readMediaDrag(event: DragEvent) {
  const custom = event.dataTransfer.getData(MEDIA_MIME)
  if (custom) return custom
  const text = event.dataTransfer.getData('text/plain') || event.dataTransfer.getData('Text')
  return text.startsWith(MEDIA_PREFIX) ? text.slice(MEDIA_PREFIX.length) : ''
}

function fileUris(data: DataTransfer | null) {
  if (!data) return []
  return data
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

function isFinderDrop(data: DataTransfer | null) {
  if (!data) return false
  const types = [...data.types]
  if (types.includes(MEDIA_MIME)) return false
  return types.includes('Files') || types.includes('text/uri-list')
}

function readFinderPathsNow() {
  try {
    const xhr = new XMLHttpRequest()
    xhr.open('GET', '/api/media/finder-drag', false)
    xhr.send()
    if (xhr.status !== 200) return []
    const parsed = JSON.parse(xhr.responseText) as unknown
    return Array.isArray(parsed) ? parsed.filter((path) => typeof path === 'string') : []
  } catch {
    return []
  }
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
  const { stars, toggle } = useStars()
  const layers = state?.layers ?? []
  const [presetName, setPresetName] = useState('')
  const cards = useRef<Array<HTMLDivElement | null>>([])
  const dragRef = useRef<LayerDrag | null>(null)
  const [drag, setDrag] = useState<LayerDrag | null>(null)
  const [dropAt, setDropAt] = useState<number | 'new' | null>(null)
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

  function dropMedia(event: DragEvent, index: number | 'new') {
    setDropAt(null)
    const id = readMediaDrag(event)
    if (!id) return
    event.preventDefault()
    event.stopPropagation()
    if (index === 'new') send({ type: 'add_library_media', ids: [id] })
    else send({ type: 'replace_library_media', layer: index, id })
  }

  return (
    <div className={`flex h-full min-h-0 flex-1 flex-col gap-2 ${drag ? 'select-none' : ''}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5">
          <span className="text-sm font-medium">Layers</span>
          <SectionInfo
            label="About layers"
            text="Click a layer to choose what Effects and Picture edit. That click does not change the picture. Opacity, blend, and the enable switch do. Drag the grip to reorder; the top of the list is in front. Drop a library item on a layer to replace it, or on the dashed row to add a layer behind the others. A new layer starts off, at 40% opacity. This works from a second window on this same page. Solo shows only the selected layer and is not saved. The star remembers a layer or its effect on this computer."
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
      <div className="flex items-center gap-2">
        <Input
          value={presetName}
          placeholder="Name this composition"
          aria-label="Preset name"
          onChange={(event) => setPresetName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && presetName.trim()) {
              send({ type: 'save_preset', name: presetName.trim() })
              setPresetName('')
            }
          }}
        />
        <Button
          disabled={!presetName.trim()}
          onClick={() => {
            send({ type: 'save_preset', name: presetName.trim() })
            setPresetName('')
          }}
        >
          Save as preset
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Save scene in the header writes the open scene under its current name. This stores a new named copy of these layers.
      </p>
      <div
        className={`rounded-xl border border-dashed px-3 py-2 text-center text-xs ${dropAt === 'new' ? 'border-primary bg-primary/15 text-foreground' : 'border-border text-muted-foreground'}`}
        onDragEnter={(event) => takeMedia(event, 'new')}
        onDragOver={(event) => takeMedia(event, 'new')}
        onDragLeave={leaveMedia}
        onDrop={(event) => dropMedia(event, 'new')}
      >
        Drop a library item here to add a layer. It starts off, at 40%.
      </div>
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
                {layer.effect_name && !layer.locked ? (
                  <button
                    type="button"
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={(event) => {
                      event.stopPropagation()
                      send({ type: 'clear_layer_effect', index })
                    }}
                  >
                    Remove effect
                  </button>
                ) : null}
                <StarMark
                  on={stars.includes(starKeyForLayer(layer))}
                  label={layer.effect_name || layer.media_name || layer.name}
                  onToggle={() => toggle(starKeyForLayer(layer))}
                />
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
                  className="data-unchecked:!bg-red-600 data-checked:!bg-green-600"
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
              disabled={!layer.effect_name || layer.locked}
              onSelect={() => send({ type: 'clear_layer_effect', index })}
            >
              Remove effect
            </ContextMenuItem>
            <ContextMenuItem
              variant="destructive"
              disabled={removeLocked}
              onSelect={() => send({ type: 'remove_layer', index })}
            >
              Remove layer
            </ContextMenuItem>
          </ContextMenuContent>
          </ContextMenu>
        )
      })}
      </div>
    </div>
  )
}

export function EffectsPanel() {
  const { state, send } = useFosfora()
  const { stars, toggle } = useStars()
  const layer = state ? activeLayer(state.layers, state.active_layer) : undefined
  const [query, setQuery] = useState('')
  const effects = (state?.effects ?? [])
    .filter((effect) => effect.name.toLowerCase().includes(query.trim().toLowerCase()))
    .sort((left, right) => {
      const leftStar = stars.includes(`effect:${left.name}`) ? 0 : 1
      const rightStar = stars.includes(`effect:${right.name}`) ? 0 : 1
      return leftStar - rightStar
    })

  return (
    <div className="grid gap-4 lg:grid-cols-[280px_1fr]">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-1.5">
            Effects
            <SectionInfo
              label="About effects"
              text="Loads onto the layer you selected on Layers. The highlighted name is the effect already on that layer. Choosing one replaces the look on that layer and leaves the other layers as they are. Star an effect to keep it at the top of this list."
            />
          </CardTitle>
          <CardDescription>Loads onto {layer?.name ?? 'the selected layer'}.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Input value={query} placeholder="Search effects" onChange={(event) => setQuery(event.target.value)} />
          <ScrollArea className="h-[28rem]">
            <div className="flex flex-col gap-1 pr-3">
              {effects.map((effect) => (
                <div key={effect.index} className="flex items-center gap-1">
                  <Button
                    variant={layer?.effect_name === effect.name ? 'secondary' : 'ghost'}
                    className="min-w-0 flex-1 justify-start"
                    onClick={() => send({ type: 'load_effect', index: effect.index })}
                  >
                    <span className="truncate">{effect.name}</span>
                  </Button>
                  <StarMark
                    on={stars.includes(`effect:${effect.name}`)}
                    label={effect.name}
                    onToggle={() => toggle(`effect:${effect.name}`)}
                  />
                </div>
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

type LibraryKind = 'all' | 'image' | 'video'

export function LibraryPanel() {
  const { state, send } = useFosfora()
  const [query, setQuery] = useState('')
  const [kindFilter, setKindFilter] = useState<LibraryKind>('all')
  const [picked, setPicked] = useState<string[]>([])
  const [renaming, setRenaming] = useState<string | null>(null)
  const [nameMenu, setNameMenu] = useState<{ id: string; x: number; y: number } | null>(null)
  const renameOnce = useRef(false)
  const [note, setNote] = useState('')
  const [finderOver, setFinderOver] = useState(false)
  const heldPaths = useRef<string[]>([])
  const sendRef = useRef(send)
  sendRef.current = send
  const uploadRef = useRef<(files: FileList | null) => Promise<void>>(async () => {})
  const items = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return (state?.library ?? [])
      .filter((item) => {
        if (kindFilter !== 'all' && item.kind !== kindFilter) return false
        return !needle || item.name.toLowerCase().includes(needle)
      })
      .slice()
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }))
  }, [kindFilter, query, state?.library])

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
  uploadRef.current = upload

  useEffect(() => {
    if (!nameMenu) return
    function close() {
      setNameMenu(null)
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === 'Escape') close()
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('keydown', onKey)
    }
  }, [nameMenu])

  useEffect(() => {
    let reading = false
    function remember(paths: string[]) {
      if (paths.length > 0) heldPaths.current = paths
    }
    function pull() {
      if (reading) return
      reading = true
      fetch('/api/media/finder-drag')
        .then((response) => (response.ok ? response.json() : []))
        .then((paths) => {
          remember(Array.isArray(paths) ? paths.filter((path) => typeof path === 'string') : [])
        })
        .catch(() => [])
        .finally(() => {
          reading = false
        })
    }
    function over(event: globalThis.DragEvent) {
      if (!isFinderDrop(event.dataTransfer)) return
      event.preventDefault()
      if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
      setFinderOver(true)
      pull()
    }
    function enter(event: globalThis.DragEvent) {
      if (!isFinderDrop(event.dataTransfer)) return
      if (event.relatedTarget == null) heldPaths.current = []
      over(event)
    }
    function leave(event: globalThis.DragEvent) {
      if (event.relatedTarget == null) setFinderOver(false)
    }
    function drop(event: globalThis.DragEvent) {
      if (!isFinderDrop(event.dataTransfer)) return
      event.preventDefault()
      setFinderOver(false)
      const paths = [
        ...new Set([...fileUris(event.dataTransfer), ...heldPaths.current, ...readFinderPathsNow()]),
      ]
      heldPaths.current = []
      if (paths.length > 0) {
        sendRef.current({ type: 'link_library_files', paths })
        setNote(
          paths.length === 1
            ? 'Adding that file to the library. It stays in Finder and is not copied.'
            : `Adding ${paths.length} items to the library. They stay in Finder and are not copied.`,
        )
        return
      }
      // The browser drop includes the file bytes and usually not the Finder
      // path. NSDragPboard is often already cleared by the time this runs.
      const files = event.dataTransfer?.files
      if (files && files.length > 0) {
        setNote(
          files.length === 1
            ? 'Finder did not share the file location, so this drop is being copied into the library.'
            : `Finder did not share the file locations, so ${files.length} files are being copied into the library.`,
        )
        void uploadRef.current(files)
        return
      }
      setNote('The Finder location was not available, so nothing was added and nothing was copied.')
    }
    window.addEventListener('dragenter', enter)
    window.addEventListener('dragover', over)
    window.addEventListener('dragleave', leave)
    window.addEventListener('drop', drop)
    return () => {
      window.removeEventListener('dragenter', enter)
      window.removeEventListener('dragover', over)
      window.removeEventListener('dragleave', leave)
      window.removeEventListener('drop', drop)
    }
  }, [])

  return (
    <Card className={`relative min-h-full ${finderOver ? 'ring-2 ring-primary' : ''}`}>
      {finderOver ? (
        <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-xl bg-primary/15 text-sm font-medium">
          Drop to add to the library.
        </div>
      ) : null}
      <CardHeader>
        <CardTitle className="flex items-center gap-1.5">
          Library
          <SectionInfo
            label="About the library"
            text="Drag a file or folder from Finder onto this page. When Finder shares the location, the file stays where it is and nothing is copied. When it does not, the drop is copied into the library, the same as Upload. Filter the table to images or videos. Names stay in alphabetical order. Right-click a row to rename the file; scenes that use it follow the new name. Drop a library item on the Layers page to replace a layer, or on the dashed row to add one. A new layer starts off, at 40% opacity. The trash icon hides it from this list and leaves the original file where it is. A file a scene still uses stays in the list."
          />
        </CardTitle>
        <CardDescription>
          Images and videos, A to Z. Right-click a row to rename it. A Finder drop stays put when the location is shared, and is copied when it is not. Upload copies into the show.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap gap-2">
          <Input className="max-w-xs" value={query} placeholder="Search media" onChange={(event) => setQuery(event.target.value)} />
          <div className="flex gap-1" role="group" aria-label="Show images or videos">
            {(
              [
                ['all', 'All'],
                ['image', 'Images'],
                ['video', 'Videos'],
              ] as const
            ).map(([id, label]) => (
              <Button key={id} size="sm" variant={kindFilter === id ? 'secondary' : 'outline'} onClick={() => setKindFilter(id)}>
                {label}
              </Button>
            ))}
          </div>
          <Button variant="outline" asChild>
            <label>
              Upload
              <input
                className="hidden"
                type="file"
                multiple
                accept="video/mp4,video/quicktime,video/webm,image/png,image/jpeg,image/webp"
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
        <div className="max-h-[32rem] overflow-auto rounded-lg ring-1 ring-foreground/10">
          <table className="w-full border-collapse text-left text-xs">
            <thead className="sticky top-0 z-10 bg-card text-muted-foreground">
              <tr className="border-b border-foreground/10">
                <th className="w-12 px-2 py-1.5 font-medium" />
                <th className="px-2 py-1.5 font-medium" aria-sort="ascending">
                  Name
                </th>
                <th className="w-16 px-2 py-1.5 font-medium">Kind</th>
                <th className="px-2 py-1.5 font-medium">Details</th>
                <th className="w-8 px-2 py-1.5 font-medium" />
              </tr>
            </thead>
            <tbody>
              {items.length === 0 ? (
                <tr>
                  <td colSpan={5} className="px-2 py-6 text-center text-muted-foreground">
                    {kindFilter === 'image' ? 'No images' : kindFilter === 'video' ? 'No videos' : 'No media'}
                    {query.trim() ? ' match that search' : ''}.
                  </td>
                </tr>
              ) : null}
              {items.map((item) => {
                const on = picked.includes(item.id)
                const editing = renaming === item.id
                const kindLabel = item.kind === 'image' ? 'Image' : item.kind === 'video' ? 'Video' : item.kind || 'File'
                const detail = [
                  item.location === 'linked' ? item.path : '',
                  item.scenes && item.scenes.length > 0 ? `In ${item.scenes.join(', ')}` : '',
                  item.status && item.status !== 'ok' ? item.detail || item.status : '',
                ]
                  .filter(Boolean)
                  .join('\n')
                function commitName(value: string) {
                  if (renameOnce.current) return
                  renameOnce.current = true
                  const next = value.trim()
                  setRenaming(null)
                  if (next && next !== item.name) send({ type: 'rename_library_item', id: item.id, name: next })
                  queueMicrotask(() => {
                    renameOnce.current = false
                  })
                }
                return (
                  <tr
                    key={item.id}
                    draggable={!editing}
                    onContextMenu={(event) => {
                      event.preventDefault()
                      setNameMenu({ id: item.id, x: event.clientX, y: event.clientY })
                    }}
                    title={detail || undefined}
                    className={`border-b border-foreground/10 ${editing ? '' : 'cursor-grab active:cursor-grabbing'} ${on ? 'bg-primary/15' : 'hover:bg-muted/60'}`}
                    onDragStart={(event) => writeMediaDrag(event, item.id)}
                    onClick={() => {
                      if (editing) return
                      setPicked((current) =>
                        current.includes(item.id) ? current.filter((id) => id !== item.id) : [...current, item.id],
                      )
                    }}
                  >
                    <td className="px-2 py-1">
                      {item.thumb ? (
                        <img
                          src={`/api/media/thumb/${encodeURIComponent(item.thumb)}`}
                          alt=""
                          draggable={false}
                          className="h-6 w-10 rounded-sm bg-muted object-cover"
                        />
                      ) : (
                        <span className="block h-6 w-10 rounded-sm bg-muted" />
                      )}
                    </td>
                    <td className="max-w-48 px-2 py-1">
                      {editing ? (
                        <input
                          autoFocus
                          defaultValue={item.name}
                          aria-label={`Rename ${item.name}`}
                          className="w-full min-w-0 rounded-sm bg-background px-1 py-0.5 text-xs outline-none ring-1 ring-ring"
                          onClick={(event) => event.stopPropagation()}
                          onMouseDown={(event) => event.stopPropagation()}
                          onFocus={(event) => {
                            const value = event.currentTarget.value
                            const dot = value.lastIndexOf('.')
                            event.currentTarget.setSelectionRange(0, dot > 0 ? dot : value.length)
                          }}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter') {
                              event.preventDefault()
                              commitName(event.currentTarget.value)
                            } else if (event.key === 'Escape') {
                              renameOnce.current = true
                              setRenaming(null)
                              queueMicrotask(() => {
                                renameOnce.current = false
                              })
                            }
                          }}
                          onBlur={(event) => commitName(event.currentTarget.value)}
                        />
                      ) : (
                        <span className="block truncate font-medium">{item.name}</span>
                      )}
                      {item.status && item.status !== 'ok' ? (
                        <span className="block truncate text-destructive">{item.detail || item.status}</span>
                      ) : null}
                    </td>
                    <td className="px-2 py-1 text-muted-foreground">
                      <Badge variant={item.status && item.status !== 'ok' ? 'destructive' : 'outline'}>{kindLabel}</Badge>
                    </td>
                    <td className="max-w-40 truncate px-2 py-1 text-muted-foreground">{libraryMeta(item)}</td>
                    <td className="px-2 py-1 text-right">
                      <button
                        type="button"
                        aria-label={`Remove ${item.name}`}
                        title={
                          item.scenes && item.scenes.length > 0
                            ? `Used by ${item.scenes.join(', ')}`
                            : `Hide ${item.name} from the library. The file stays where it is.`
                        }
                        className="text-muted-foreground hover:text-destructive"
                        onClick={(event) => {
                          event.stopPropagation()
                          send({ type: 'remove_library_item', id: item.id })
                        }}
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
        {nameMenu
          ? createPortal(
              <div
                className="fixed z-50 min-w-40 rounded-lg bg-popover p-1 text-sm text-popover-foreground shadow-md ring-1 ring-foreground/10"
                style={{ left: nameMenu.x, top: nameMenu.y }}
                onPointerDown={(event) => event.stopPropagation()}
              >
                <button
                  type="button"
                  className="flex w-full rounded-md px-1.5 py-1 text-left hover:bg-accent"
                  onClick={() => {
                    renameOnce.current = false
                    setRenaming(nameMenu.id)
                    setNameMenu(null)
                  }}
                >
                  Rename
                </button>
              </div>,
              document.body,
            )
          : null}
      </CardContent>
    </Card>
  )
}

export function ShowPanel() {
  const { state, send } = useFosfora()
  const show = state?.show
  const grade = Math.round((show?.grade_strength ?? 0.75) * 100)
  const [pendingRemove, setPendingRemove] = useState<string | null>(null)

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
              text="The show clock moves scenes and palettes on its own after Start. Each cue is an absolute time in show.json. A scene is a saved preset: the Scenes list is every saved look, and the clock is the schedule of those looks. Prev and next step the scene or palette by hand. Blackout cuts the output. Palette auto lets the schedule choose colours again. Right-click a palette to rename it. Right-click a scene, or use its trash icon, to remove it. Built-in scenes stay."
            />
          </CardTitle>
          <CardDescription>{show?.validation || 'Open a show pack in Fosfora.'}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex justify-between text-sm">
            <span>{show?.scene || 'Scene'}</span>
            <span className="text-muted-foreground">
              {show?.next_scene
                ? `next ${show.next_scene}${show.next_scene_secs != null ? ` at ${clock(show.next_scene_secs * 1000)}` : ''}`
                : ''}
            </span>
          </div>
          <div className="flex justify-between text-sm">
            <span>{show?.palette || 'Palette'}</span>
            <Badge variant="secondary">{show?.palette_manual ? 'Manual' : 'Auto'}</Badge>
          </div>
          <div className="flex flex-wrap gap-2">
            {(show?.palettes ?? []).map((id) => {
              const strip = show?.palette_colors?.find((entry) => entry.id === id)
              return (
                <ContextMenu key={id}>
                  <ContextMenuTrigger asChild>
                    <div className="flex w-40 flex-col gap-1">
                      <div className="flex h-1.5 overflow-hidden rounded-full bg-muted">
                        {(strip?.colors ?? []).map((rgba, swatch) => (
                          <span key={swatch} className="h-full flex-1" style={{ background: swatchCss(rgba) }} />
                        ))}
                      </div>
                      <Button
                        size="sm"
                        variant={show?.palette === id ? 'default' : 'outline'}
                        onClick={() => showAction('select_palette', { id })}
                      >
                        <span className="truncate">{id}</span>
                      </Button>
                    </div>
                  </ContextMenuTrigger>
                  <ContextMenuContent>
                    <ContextMenuItem
                      onSelect={() => {
                        const next = window.prompt('Rename palette', id)
                        if (!next || next.trim() === id) return
                        send({ type: 'rename_palette', from: id, to: next.trim() })
                      }}
                    >
                      Rename
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              )
            })}
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
            <Button
              className={show?.transport !== 'running' && show?.transport !== 'paused' ? '!bg-green-700 !text-white hover:!bg-green-600' : ''}
              variant={show?.transport !== 'running' && show?.transport !== 'paused' ? 'default' : 'outline'}
              onClick={() => showAction('start')}
            >
              <Play />
              Start
            </Button>
            <Button
              className={show?.transport === 'running' ? '!bg-amber-600 !text-white hover:!bg-amber-500' : ''}
              variant={show?.transport === 'running' ? 'default' : 'outline'}
              onClick={() => showAction('pause')}
            >
              <Pause />
              Pause
            </Button>
            <Button
              className={show?.transport === 'paused' ? '!bg-green-700 !text-white hover:!bg-green-600' : ''}
              variant={show?.transport === 'paused' ? 'default' : 'outline'}
              onClick={() => showAction('resume')}
            >
              <Play />
              Resume
            </Button>
            <Button className="!bg-red-700 !text-white hover:!bg-red-600" onClick={() => showAction('stop_auto')}>
              <Square />
              Stop
            </Button>
            <Button variant="outline" onClick={() => showAction('reset')}>Back to start</Button>
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
            Scenes
            <SectionInfo
              label="About scenes"
              text="A scene and a preset are the same saved look. The highlighted name is the one Fosfora has applied. Right-click a name to rename or remove it, or use the trash icon. Built-in scenes stay. Removing a scene takes it off the show clock if it was scheduled. The picture stays as it is. In Perform, switching scenes waits until you save, discard, or cancel."
            />
          </CardTitle>
          <CardDescription>Right-click a scene to rename or remove it. Built-in scenes stay.</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="max-h-[28rem] overflow-auto">
            <div className="flex flex-col gap-1 pr-3">
              {(state?.presets ?? []).map((preset) => {
                const applied = state?.applied_preset === preset.name
                const loading = state?.loading_preset === preset.name
                const waiting =
                  state?.requested_preset === preset.name && !applied && !loading
                const confirming = pendingRemove === preset.name
                return (
                  <div key={preset.index} className="flex flex-col gap-1">
                    <ContextMenu modal={false}>
                      <ContextMenuTrigger asChild>
                        <div className="flex items-center gap-1">
                          <Button
                            variant={applied ? 'secondary' : 'ghost'}
                            className="min-w-0 flex-1 justify-start"
                            onClick={() => send({ type: 'load_preset', index: preset.index })}
                          >
                            <span className="truncate">
                              {preset.name}
                              {loading ? ' · loading' : ''}
                              {waiting ? ' · queued' : ''}
                            </span>
                            {preset.builtin ? <Badge variant="outline">Built-in</Badge> : null}
                          </Button>
                          {preset.builtin ? null : confirming ? (
                            <>
                              <Button
                                size="sm"
                                variant="destructive"
                                onClick={() => {
                                  send({ type: 'delete_preset', name: preset.name })
                                  setPendingRemove(null)
                                }}
                              >
                                Remove
                              </Button>
                              <Button size="sm" variant="ghost" onClick={() => setPendingRemove(null)}>
                                Cancel
                              </Button>
                            </>
                          ) : (
                            <Button
                              variant="ghost"
                              size="icon"
                              aria-label={`Remove ${preset.name}`}
                              title="Remove this scene. The picture stays."
                              onClick={() => setPendingRemove(preset.name)}
                            >
                              <Trash2 />
                            </Button>
                          )}
                        </div>
                      </ContextMenuTrigger>
                      <ContextMenuContent collisionPadding={8}>
                        <ContextMenuItem
                          disabled={preset.builtin}
                          onSelect={() => {
                            const next = window.prompt('Rename scene', preset.name)
                            if (!next || next.trim() === preset.name) return
                            send({ type: 'rename_preset', from: preset.name, to: next.trim() })
                          }}
                        >
                          Rename
                        </ContextMenuItem>
                        <ContextMenuItem
                          variant="destructive"
                          disabled={preset.builtin}
                          onSelect={() => setPendingRemove(preset.name)}
                        >
                          {preset.builtin ? 'Built-in scenes stay' : 'Remove'}
                        </ContextMenuItem>
                      </ContextMenuContent>
                    </ContextMenu>
                  </div>
                )
              })}
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

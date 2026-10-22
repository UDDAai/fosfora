import { useState } from 'react'
import { Slider } from '@/components/ui/slider'

type HeldSliderProps = {
  value: number
  min?: number
  max?: number
  step?: number
  disabled?: boolean
  onChange: (value: number) => void
}

/** Keeps the thumb where the operator left it while a state broadcast arrives. */
export function HeldSlider({
  value,
  min = 0,
  max = 100,
  step = 1,
  disabled,
  onChange,
}: HeldSliderProps) {
  const [held, setHeld] = useState<number | null>(null)
  const shown = held ?? value
  return (
    <Slider
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      value={[shown]}
      onValueChange={(next) => {
        const value = next[0]
        if (value == null) return
        setHeld(value)
        onChange(value)
      }}
      onValueCommit={() => setHeld(null)}
    />
  )
}

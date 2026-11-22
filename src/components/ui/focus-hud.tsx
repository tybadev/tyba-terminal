import * as React from "react"
import { Pause, Play, Stop } from "@phosphor-icons/react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

// FocusHUD do Muriki (`FocusHUD`, `.mk-hud`, quadro Main): pílula de vidro
// flutuante de 56 de altura, raio 14, 0 6 0 20 de folga e gap 12; ponto
// âmbar de 12 pulsando, tempo mono 20/600, tarefa 13px apagada até 200 com
// reticências, fio de 1×24, pausar no botão de vidro (continuar no
// primário) e encerrar no fantasma. Pausado: ponto apagado, tempo apagado,
// sem pulso. O estado vem de fora (o core); o componente não guarda nada.
function FocusHUD({
  time,
  task,
  paused = false,
  onPause,
  onStop,
  className,
  ...props
}: Omit<React.ComponentProps<"div">, "children"> & {
  time: string
  task?: React.ReactNode
  paused?: boolean
  /** Sem `onPause`, sem o botão de pausar (Windows). */
  onPause?: () => void
  onStop?: () => void
}) {
  return (
    <div
      role="status"
      aria-label={paused ? "Foco pausado" : "Foco em andamento"}
      data-slot="focus-hud"
      data-state={paused ? "paused" : "running"}
      className={cn(
        "tyba-mk-glass is-float inline-flex h-14 items-center gap-3 rounded-[14px] border border-[var(--tyba-mk-glass-edge,var(--tyba-border))] bg-tyba-overlay/95 pr-1.5 pl-5 [box-shadow:var(--tyba-edge),var(--tyba-shadow-lg)] backdrop-blur-xl",
        className,
      )}
      {...props}
    >
      <span
        aria-hidden
        className={cn(
          "tyba-hud-dot relative size-3 shrink-0 rounded-full",
          paused
            ? "bg-[rgba(10,24,48,0.2)]"
            : "is-running bg-[var(--tyba-spark,var(--tyba-amber-fill))] shadow-[0_0_0_1px_rgba(10,24,48,0.12)]",
        )}
      />
      <span
        className={cn(
          "font-mono text-[20px] font-semibold tracking-[-0.02em] tabular-nums",
          paused ? "text-tyba-text-muted" : "text-tyba-text",
        )}
      >
        {time}
      </span>
      {task && (
        <span className="max-w-[200px] truncate text-[13px] text-tyba-text-muted">{task}</span>
      )}
      <span aria-hidden className="mx-0.5 h-6 w-px bg-[var(--tyba-mk-glass-line,var(--tyba-line))]" />
      {onPause && (
        <Button
          variant={paused ? "default" : "outline"}
          size="icon"
          aria-label={paused ? "Continuar" : "Pausar"}
          onClick={onPause}
        >
          {paused ? <Play /> : <Pause />}
        </Button>
      )}
      <Button variant="ghost" size="icon" aria-label="Encerrar foco" onClick={onStop}>
        <Stop />
      </Button>
    </div>
  )
}

export { FocusHUD }

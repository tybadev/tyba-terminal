import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"
import {
  ArrowRight,
  CheckCircle,
  Info,
  Warning,
  WarningCircle,
  type Icon as PhosphorIcon,
} from "@phosphor-icons/react"

import { cn } from "@/lib/utils"

const notificationVariants = cva(
  "rounded-md border border-tyba-border px-4 py-3 text-sm text-tyba-text",
  {
    variants: {
      variant: {
        info: "[--notification-accent:var(--tyba-blue)]",
        success: "[--notification-accent:var(--tyba-green)]",
        warning: "[--notification-accent:var(--tyba-amber)]",
        danger: "[--notification-accent:var(--tyba-red)]",
        neutral: "[--notification-accent:var(--tyba-text-faint)]",
      },
    },
    defaultVariants: {
      variant: "info",
    },
  }
)

type NotificationVariant = NonNullable<
  VariantProps<typeof notificationVariants>["variant"]
>

const variantMeta: Record<
  NotificationVariant,
  { Icon: PhosphorIcon; role: "status" | "alert" }
> = {
  info: { Icon: Info, role: "status" },
  success: { Icon: CheckCircle, role: "status" },
  warning: { Icon: Warning, role: "alert" },
  danger: { Icon: WarningCircle, role: "alert" },
  neutral: { Icon: Info, role: "status" },
}

function Notification({
  className,
  variant = "info",
  icon,
  children,
  ...props
}: React.ComponentProps<"div"> &
  VariantProps<typeof notificationVariants> & { icon?: React.ReactNode }) {
  const { Icon, role } = variantMeta[variant ?? "info"]

  return (
    <div
      data-slot="notification"
      data-variant={variant}
      role={role}
      aria-live={role === "status" ? "polite" : undefined}
      className={cn(notificationVariants({ variant }), className)}
      {...props}
    >
      <div className="flex gap-3">
        <span
          aria-hidden="true"
          className="mt-0.5 shrink-0 text-[var(--notification-accent)]"
        >
          {icon ?? <Icon size={16} />}
        </span>
        <div className="flex grow flex-wrap justify-between gap-x-3 gap-y-1">
          {children}
        </div>
      </div>
    </div>
  )
}

function NotificationTitle({
  className,
  ...props
}: React.ComponentProps<"p">) {
  return (
    <p
      data-slot="notification-title"
      className={cn("font-medium", className)}
      {...props}
    />
  )
}

function NotificationDescription({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="notification-description"
      className={cn(
        "w-full text-tyba-text-muted [&_ul]:list-inside [&_ul]:list-disc [&_ul]:space-y-0.5",
        className
      )}
      {...props}
    />
  )
}

function NotificationLink({
  className,
  asChild = false,
  children,
  ...props
}: React.ComponentProps<"a"> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "a"

  return (
    <Comp
      data-slot="notification-link"
      className={cn(
        "group rounded-sm whitespace-nowrap font-medium hover:underline focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)]",
        className
      )}
      {...props}
    >
      {children}
      {/* com asChild o caller compõe o conteúdo inteiro (Slot exige filho único) */}
      {!asChild && (
        <ArrowRight
          aria-hidden="true"
          size={16}
          className="-mt-0.5 ms-1 inline-flex opacity-60 transition-transform motion-safe:group-hover:translate-x-0.5"
        />
      )}
    </Comp>
  )
}

export {
  Notification,
  NotificationTitle,
  NotificationDescription,
  NotificationLink,
  notificationVariants,
}

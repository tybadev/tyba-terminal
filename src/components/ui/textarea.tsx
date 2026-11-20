import * as React from "react"

import { cn } from "@/lib/utils"
import { FIELD_SURFACE } from "./input"

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      autoCapitalize="off"
      autoCorrect="off"
      spellCheck={false}
      data-slot="textarea"
      className={cn(FIELD_SURFACE, "w-full min-w-0 resize-y px-4 py-2.5", className)}
      {...props}
    />
  )
}

export { Textarea }

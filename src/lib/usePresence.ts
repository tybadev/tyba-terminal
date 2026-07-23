import { useEffect, useRef, useState } from "react";

export interface Presence {
  mounted: boolean;
  exiting: boolean;
}

export function usePresence(present: boolean, exitMs: number): Presence {
  const [mounted, setMounted] = useState(present);
  const [exiting, setExiting] = useState(false);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    if (present) {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
      setExiting(false);
      setMounted(true);
      return;
    }
    if (!mounted) return;
    setExiting(true);
    if (exitMs <= 0) {
      setMounted(false);
      setExiting(false);
      return;
    }
    timer.current = window.setTimeout(() => {
      setMounted(false);
      setExiting(false);
      timer.current = null;
    }, exitMs);
    return () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
    };
  }, [present, exitMs, mounted]);

  return { mounted, exiting };
}

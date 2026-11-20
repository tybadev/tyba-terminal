export const IS_MAC = navigator.platform.toUpperCase().includes("MAC");

/** Windows: sem SIGSTOP, o FocusHUD não oferece pausar. */
export const IS_WINDOWS = navigator.platform.toUpperCase().startsWith("WIN");

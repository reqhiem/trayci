export const IPC = {
  usageGet: "usage:get-state",
  usageRefresh: "usage:refresh-all",
  usageChanged: "usage:state-changed",
  settingsGet: "settings:get",
  settingsUpdate: "settings:update",
  appHide: "app:hide-popover",
  appResize: "app:resize-popover",
  appQuit: "app:quit",
} as const;

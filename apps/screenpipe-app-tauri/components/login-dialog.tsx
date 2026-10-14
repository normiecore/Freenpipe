import { create } from "zustand";

// screenpipe showed a "login required" dialog here before cloud-only actions.
// Freenpipe has no hosted account, so nothing ever needs a login: the dialog
// renders nothing and `checkLogin` always lets the caller proceed. The exports
// keep their shape so existing mounts and callers compile unchanged.
export function LoginDialog() {
  return null;
}

interface LoginDialogState {
  isOpen: boolean;
  setIsOpen: (open: boolean) => void;
  checkLogin: (user: any | null, showDialog?: boolean) => boolean;
}

export const useLoginDialog = create<LoginDialogState>(() => ({
  isOpen: false,
  setIsOpen: () => {},
  checkLogin: () => true,
}));

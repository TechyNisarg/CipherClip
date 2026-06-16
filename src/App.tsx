import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { motion, AnimatePresence } from "framer-motion";
import { QRCodeSVG } from 'qrcode.react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { enable, isEnabled, disable } from '@tauri-apps/plugin-autostart';
import { Copy, MonitorSmartphone, ShieldCheck, Clock, Trash2, Pin, SlidersHorizontal, X, Check, AlertTriangle, RefreshCcw, ArrowLeft, Network, Key, Maximize2, Loader2, Scan, QrCode, Plus } from "lucide-react";
import { scan, cancel, Format, requestPermissions } from '@tauri-apps/plugin-barcode-scanner';
import { writeText as writeTextToClipboard, readText, writeImage } from '@tauri-apps/plugin-clipboard-manager';
import { type as osType } from '@tauri-apps/plugin-os';

const isMobile = osType() === 'android' || osType() === 'ios';

interface ClipItem {
  id: number;
  content_type: string;
  content: string;
  timestamp: number;
  pinned: boolean;
  is_locked: boolean;
}

function App() {
  const [clips, setClips] = useState<ClipItem[]>([]);
  const [loading, setLoading] = useState(true);

  const [limit, setLimit] = useState(100);
  const [shortcut, setShortcut] = useState("Super+Shift+C");
  const [shortcutInput, setShortcutInput] = useState("Super+Shift+C");
  const [shortcutSaved, setShortcutSaved] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showConfirmClear, setShowConfirmClear] = useState(false);
  const [deleteLocked, setDeleteLocked] = useState(false);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [connectedPeers, setConnectedPeers] = useState<{ip: string, name: string}[]>([]);
  const [activeTab, setActiveTab] = useState<'recent' | 'pinned'>('recent');
  const [showTutorial, setShowTutorial] = useState(false);
  
  useEffect(() => {
    if (!localStorage.getItem('hasSeenTutorial')) {
      setShowTutorial(true);
    }
  }, []);

  const closeTutorial = () => {
    localStorage.setItem('hasSeenTutorial', 'true');
    setShowTutorial(false);
  };
  
  const [deletedClips, setDeletedClips] = useState<ClipItem[]>([]);
  const [showRecycleBin, setShowRecycleBin] = useState(false);
  
  const [showNetworkSync, setShowNetworkSync] = useState(false);
  const [showConnectedDevicesModal, setShowConnectedDevicesModal] = useState(false);
  const [showPairing, setShowPairing] = useState(false);
  const [syncKey, setSyncKey] = useState("");
  const [syncKeyInput, setSyncKeyInput] = useState("");
  const [alertModal, setAlertModal] = useState<{message: string, isError: boolean} | null>(null);
  const [showEncryptionModal, setShowEncryptionModal] = useState(false);
  const [showConfirmEmpty, setShowConfirmEmpty] = useState(false);
  const [clipToDelete, setClipToDelete] = useState<ClipItem | null>(null);
  const [limitSaved, setLimitSaved] = useState(false);
  const [copiedKey, setCopiedKey] = useState(false);
  const [autoStart, setAutoStart] = useState(false);
  const [theme, setTheme] = useState("system");
  const [hasMasterPassword, setHasMasterPassword] = useState(false);
  const [showPasswordSetup, setShowPasswordSetup] = useState(false);
  const [showPasswordPrompt, setShowPasswordPrompt] = useState<{clipId: number, isAutoPaste: boolean, action: 'copy' | 'unlock'} | null>(null);
  const [pendingLockId, setPendingLockId] = useState<number | null>(null);
  const [passwordInput, setPasswordInput] = useState("");
  const [settingsTab, setSettingsTab] = useState<'general' | 'sync' | 'data' | 'about'>('general');
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    if (settingsTab === 'sync' || showNetworkSync) {
      setSyncKeyInput(syncKey);
    }
  }, [settingsTab, showNetworkSync, syncKey]);

  const showToast = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2500);
  };

  const fetchHistory = useCallback(async () => {
    try {
      const history: ClipItem[] = await invoke("get_history");
      setClips(history);
    } catch (error) {
      console.error("Failed to fetch history:", error);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchDeletedHistory = async () => {
    try {
      const history: ClipItem[] = await invoke("get_deleted_clips");
      setDeletedClips(history);
    } catch (error) {
      console.error("Failed to fetch deleted history:", error);
    }
  };

  const fetchSettings = async () => {
    try {
      const s: any = await invoke("get_settings");
      if (s && s.history_limit) setLimit(s.history_limit);
      if (s && s.global_shortcut) {
        setShortcut(s.global_shortcut);
        setShortcutInput(s.global_shortcut);
      }
      if (s && s.theme) {
        setTheme(s.theme);
      }
      try {
        const hasPwd = await invoke("has_master_password");
        setHasMasterPassword(hasPwd as boolean);
      } catch (e) {
        console.warn("Failed to fetch master password status:", e);
      }
      try {
        const isAutoStartEnabled = await isEnabled();
        setAutoStart(isAutoStartEnabled);
      } catch (e) {
        console.warn("Autostart plugin error:", e);
      }
    } catch (error) {
      console.error("Failed to fetch settings:", error);
    }
  };

  const toggleAutoStart = async () => {
    try {
      if (autoStart) {
        await disable();
        setAutoStart(false);
      } else {
        await enable();
        setAutoStart(true);
      }
    } catch (err) {
      console.error("Failed to toggle autostart:", err);
      setAlertModal({ message: "Failed to toggle auto-start. Make sure your OS allows background apps.", isError: true });
    }
  };

  const [isMobile, setIsMobile] = useState(false);

  useEffect(() => {
    try {
      const t = osType();
      setIsMobile(t === 'android' || t === 'ios');
    } catch (e) {
      // ignore
    }
  }, []);

  const [isScanning, setIsScanning] = useState(false);

  const handleScanQR = async () => {
    try {
      await requestPermissions();
      setIsScanning(true);
      document.documentElement.style.backgroundColor = 'transparent';
      document.body.style.backgroundColor = 'transparent';

      // Artificial delay so the user can orient the camera before it scans instantly
      await new Promise(resolve => setTimeout(resolve, 1500));

      const result = await scan({ windowed: true, formats: [Format.QRCode] });
      
      setIsScanning(false);
      document.documentElement.style.backgroundColor = '';
      document.body.style.backgroundColor = '';

      if (result && result.content) {
        if (result.content.length === 64) {
          setSyncKeyInput(result.content);
          await invoke("set_sync_key", { hexKey: result.content });
          setSyncKey(result.content);
          setAlertModal({ message: "Successfully paired with your device! Clipboards will now sync.", isError: false });
        } else {
          setAlertModal({ message: "Invalid QR Code. Sync Key must be 64 characters long.", isError: true });
        }
      }
    } catch (err: any) {
      setIsScanning(false);
      document.documentElement.style.backgroundColor = '';
      document.body.style.backgroundColor = '';
      setAlertModal({ message: "Scanner error or cancelled.", isError: true });
    }
  };

  const handleCancelScan = async () => {
    await cancel();
    setIsScanning(false);
    document.documentElement.style.backgroundColor = '';
    document.body.style.backgroundColor = '';
  };

  const fetchSyncKey = async () => {
    try {
      const key: string = await invoke("get_sync_key");
      setSyncKey(key);
      setSyncKeyInput(key);
    } catch (err) {
      console.error("Failed to fetch sync key:", err);
    }
  };

  const updateSyncKey = async () => {
    if (!syncKeyInput.trim() || syncKeyInput.length !== 64) {
      setAlertModal({ message: "Invalid Sync Key format. It must be exactly 64 characters long.", isError: true });
      return;
    }
    try {
      await invoke("set_sync_key", { hexKey: syncKeyInput });
      setSyncKey(syncKeyInput);
      setAlertModal({ message: "Sync Key updated securely! Devices with this key will now sync payloads.", isError: false });
    } catch (err) {
      setAlertModal({ message: "Failed to update Sync Key: " + err, isError: true });
    }
  };

  useEffect(() => {
    fetchHistory();
    fetchSettings();
    fetchSyncKey();
    
    const pollPeers = async () => {
      try {
        const peers: {ip: string, name: string}[] = await invoke("get_connected_peers");
        setConnectedPeers(peers);
      } catch (error) {
        console.error(error);
      }
    };
    pollPeers();
    const peerInterval = setInterval(pollPeers, 5000);

    const unlisten = listen("clipboard-update", () => {
      fetchHistory();
    });

    const preventZoom = (e: TouchEvent) => {
      if (e.touches.length > 1) {
        e.preventDefault();
      }
    };
    document.addEventListener('touchstart', preventZoom, { passive: false });
    document.addEventListener('touchmove', preventZoom, { passive: false });

    const handleKeyDown = async (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        await invoke('hide_window');
      }
      if (e.ctrlKey && e.key.toLowerCase() === 'i') {
        e.preventDefault();
        setShowSettings(prev => !prev);
      }
      if (e.ctrlKey && e.key.toLowerCase() === 'd') {
        e.preventDefault();
        setShowConnectedDevicesModal(true);
      }
    };
    window.addEventListener('keydown', handleKeyDown);

    let unlistenFocusFn: (() => void) | undefined;
    const setupFocusListener = async () => {
      const appWindow = getCurrentWindow();
      const unlistenFocus = await appWindow.onFocusChanged(async ({ payload: focused }) => {
        const isMobileDevice = window.innerWidth < 768 || navigator.userAgent.toLowerCase().includes('android') || navigator.userAgent.toLowerCase().includes('iphone');
        if (!focused && !isMobileDevice) {
          setShowSettings(false);
          setShowRecycleBin(false);
          setShowNetworkSync(false);
          setShowConnectedDevicesModal(false);
          setShowEncryptionModal(false);
          setShowConfirmEmpty(false);
          setShowConfirmClear(false);
          setClipToDelete(null);
          setDeleteLocked(false);
          setShowPasswordSetup(false);
          setShowPasswordPrompt(null);
          setPendingLockId(null);
          setPasswordInput("");
          setAlertModal(null);
          window.scrollTo(0, 0);
        } else if (focused) {
          // Sync Mobile Clipboard to PC when app gains focus
          try {
            const t = await osType();
            if (t === 'android' || t === 'ios') {
              const text = await readText();
              if (text && text.trim().length > 0) {
                const added = await invoke("add_mobile_clip", { text });
                if (added) {
                  fetchHistory();
                }
              }
            }
          } catch (e) {
            console.error("Mobile clipboard sync error:", e);
          }
        }
      });
      unlistenFocusFn = unlistenFocus;
    };
    setupFocusListener();

    return () => {
      clearInterval(peerInterval);
      unlisten.then((f) => f());
      window.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('touchstart', preventZoom);
      document.removeEventListener('touchmove', preventZoom);
      if (unlistenFocusFn) unlistenFocusFn();
    };
  }, [fetchHistory]);

  useEffect(() => {
    const isDark = 
      theme === 'dark' || 
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    
    if (isDark) {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  }, [theme]);

  const handleCopy = async (clip: ClipItem, autoPaste: boolean = false, isUnlocked: boolean = false) => {
    if (clip.is_locked && !isUnlocked) {
      if (!hasMasterPassword) {
        setTimeout(() => setShowPasswordSetup(true), 100);
        return;
      }
      setShowPasswordPrompt({ clipId: clip.id, isAutoPaste: autoPaste, action: 'copy' });
      return;
    }
    
    try {
      if (clip.content_type === "image") {
        try {
          if (!isMobile) await invoke("set_ignore_next_update");
        } catch(e) {}
        const canvas = document.createElement('canvas');
        const img = new Image();
        img.src = `data:image/webp;base64,${clip.content}`;
        await new Promise((r) => { img.onload = r; });
        canvas.height = img.height;
        canvas.getContext('2d')?.drawImage(img, 0, 0);
        
        const pngBlob = await new Promise<Blob | null>((res) => canvas.toBlob(res, 'image/png'));
        if (pngBlob) {
          try {
            const arrayBuffer = await pngBlob.arrayBuffer();
            await writeImage(new Uint8Array(arrayBuffer));
          } catch (e) {
            console.error("Plugin writeImage failed, falling back to navigator", e);
            await navigator.clipboard.write([
              new ClipboardItem({ 'image/png': pngBlob })
            ]);
          }
        }
      } else {
        try {
          await writeTextToClipboard(clip.content);
        } catch (e) {
          // Fallback for Windows if plugin fails
          await navigator.clipboard.writeText(clip.content);
        }
      }
      setCopiedId(clip.id);
      showToast("Copied to clipboard");
      setTimeout(() => setCopiedId(null), 2000);

      if (autoPaste) {
        await invoke("hide_window");
        await invoke("paste_to_active_window");
      }
    } catch (err) {
      console.error("Failed to copy:", err);
      setAlertModal({ message: "Failed to copy to clipboard", isError: true });
    }
  };

  const executeClearHistory = async () => {
    try {
      await invoke("clear_history", { deleteLocked });
      setClips([]);
      fetchHistory(); 
      setShowConfirmClear(false);
      setShowSettings(false);
    } catch (err) {
      console.error("Failed to clear history:", err);
    }
  };

  const deleteClip = async (clip: ClipItem) => {
    if (clip.is_locked) {
      setClipToDelete(clip);
    } else {
      executeDelete(clip.id);
    }
  };

  const executeDelete = async (id: number) => {
    try {
      await invoke("delete_clip", { id });
      fetchHistory();
      if (showRecycleBin) fetchDeletedHistory();
      setClipToDelete(null);
    } catch (err) {
      console.error("Failed to delete clip:", err);
    }
  };

  const restoreClip = async (id: number) => {
    try {
      await invoke("restore_clip", { id });
      fetchDeletedHistory();
      fetchHistory();
    } catch (err) {
      console.error("Failed to restore clip:", err);
    }
  };

  const permanentlyDeleteClip = async (id: number) => {
    try {
      await invoke("permanently_delete_clip", { id });
      fetchDeletedHistory();
    } catch (err) {
      console.error("Failed to permanently delete clip:", err);
    }
  };

  const handleEmptyRecycleBin = () => {
    setShowConfirmEmpty(true);
  };

  const executeEmptyRecycleBin = async () => {
    try {
      await invoke("empty_recycle_bin");
      setDeletedClips([]);
      setShowConfirmEmpty(false);
    } catch (err) {
      console.error("Failed to empty recycle bin:", err);
    }
  };

  const togglePin = async (id: number, currentPinned: boolean) => {
    try {
      await invoke("toggle_pin", { id, pinned: !currentPinned });
      fetchHistory();
      showToast(!currentPinned ? "Moved to Pinned items" : "Unpinned item");
    } catch (err) {
      console.error("Failed to pin:", err);
    }
  };

  const toggleLock = async (id: number, currentLocked: boolean) => {
    try {
      await invoke("toggle_clip_lock", { id, isLocked: !currentLocked });
      fetchHistory();
      showToast(!currentLocked ? "Clip locked" : "Clip unlocked");
    } catch (err) {
      console.error("Failed to lock/unlock:", err);
    }
  };

  const updateLimit = async (newLimit: number) => {
    try {
      await invoke("set_limit", { limit: newLimit });
      setLimit(newLimit);
      fetchHistory();
      setLimitSaved(true);
      setTimeout(() => {
        setLimitSaved(false);
      }, 2000);
    } catch (err) {
      console.error("Failed to set limit:", err);
    }
  };

  const updateTheme = async (newTheme: string) => {
    try {
      await invoke("set_theme", { theme: newTheme });
      setTheme(newTheme);
    } catch (err) {
      console.error("Failed to set theme:", err);
    }
  };

  const executeSetMasterPassword = async () => {
    try {
      if (hasMasterPassword) {
        const isCorrect: boolean = await invoke("verify_master_password", { password: passwordInput });
        if (!isCorrect) {
          setAlertModal({ message: "Incorrect current Master Password.", isError: true });
          return;
        }
      }
      
      await invoke("set_master_password", { password: hasMasterPassword ? null : passwordInput.trim() });
      const wasRemoving = hasMasterPassword;
      setHasMasterPassword(!wasRemoving);
      setShowPasswordSetup(false);
      setPasswordInput("");
      showToast(wasRemoving ? "Password removed & clips unlocked" : "Master Password set!");
      
      if (wasRemoving) {
        fetchHistory(); 
      } else if (pendingLockId !== null) {
        await toggleLock(pendingLockId, false);
        setPendingLockId(null);
      }
    } catch (err) {
      setAlertModal({ message: "Failed to set master password: " + err, isError: true });
    }
  };

  const handleUnlockClip = async () => {
    if (!showPasswordPrompt) return;
    try {
      const isCorrect: boolean = await invoke("verify_master_password", { password: passwordInput });
      if (isCorrect) {
        if (showPasswordPrompt.action === 'copy') {
          const clipToCopy = clips.find(c => c.id === showPasswordPrompt.clipId) || deletedClips.find(c => c.id === showPasswordPrompt.clipId);
          if (clipToCopy) {
            await handleCopy(clipToCopy, showPasswordPrompt.isAutoPaste, true);
          }
        } else if (showPasswordPrompt.action === 'unlock') {
          await toggleLock(showPasswordPrompt.clipId, true);
        }
        setShowPasswordPrompt(null);
        setPasswordInput("");
      } else {
        setAlertModal({ message: "Incorrect Master Password.", isError: true });
      }
    } catch (err) {
      setAlertModal({ message: "Verification error: " + err, isError: true });
    }
  };

  const updateShortcut = async () => {
    if (!shortcutInput.trim()) {
      setShortcutInput(shortcut);
      return;
    }
    
    try {
      await invoke("set_shortcut", { shortcut: shortcutInput });
      setShortcut(shortcutInput);
      setShortcutSaved(true);
      setTimeout(() => {
        setShortcutSaved(false);
        setShowSettings(false);
      }, 2000);
    } catch (err) {
      setAlertModal({ message: String(err), isError: true });
    }
  };

  const handleShortcutKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    e.preventDefault();
    e.stopPropagation();

    const keys: string[] = [];
    if (e.ctrlKey) keys.push('Ctrl');
    if (e.altKey) keys.push('Alt');
    if (e.shiftKey) keys.push('Shift');
    if (e.metaKey) keys.push('Super');

    if (e.key === 'Backspace') {
      setShortcutInput('');
      return;
    }

    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
      setShortcutInput(keys.join('+') + (keys.length > 0 ? '+' : ''));
      return;
    }

    let key = e.key.toUpperCase();
    if (key === ' ') key = 'Space';
    if (key.length === 1 && key >= 'A' && key <= 'Z') {
      keys.push(key);
    } else if (key.startsWith('F') && key.length <= 3) {
      keys.push(key);
    } else if (['ESCAPE', 'ENTER', 'TAB', 'SPACE', 'BACKSPACE', 'DELETE'].includes(key)) {
      keys.push(key.charAt(0) + key.slice(1).toLowerCase());
    } else {
      keys.push(key);
    }

    setShortcutInput(keys.join('+'));
  };

  const hasPinned = clips.some(c => c.pinned);

  useEffect(() => {
    if (!hasPinned && activeTab === 'pinned') {
      setActiveTab('recent');
    }
  }, [hasPinned, activeTab]);

  const networkSyncUI = (
    <div className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-xl p-4 shadow-sm flex flex-col items-center w-full">
      <div className="w-full flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <Key className="w-5 h-5 text-emerald-500" />
          <label className="text-sm font-medium text-slate-700 dark:text-gray-300">
            Device Sync Key
          </label>
        </div>
        {isMobile && (
          <button
            onClick={handleScanQR}
            className="flex items-center gap-2 px-3 py-1.5 bg-indigo-50 dark:bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 hover:bg-indigo-100 dark:hover:bg-indigo-500/20 rounded-md text-xs font-medium transition-colors cursor-pointer"
          >
            <Scan className="w-4 h-4" />
            Scan QR
          </button>
        )}
      </div>
      
      <div className="flex justify-center mb-4 p-4 bg-slate-50 dark:bg-gray-100 rounded-xl w-full">
        {syncKeyInput.length === 64 ? (
          <QRCodeSVG value={syncKeyInput} size={160} />
        ) : (
          <div className="w-[160px] h-[160px] flex items-center justify-center text-slate-400 text-xs text-center border-2 border-dashed border-slate-200 dark:border-gray-300 rounded-lg">
            Enter valid 64-character key to generate QR code
          </div>
        )}
      </div>
      
      <div className="w-full relative">
        <input 
          type="text"
          inputMode="none"
          value={syncKeyInput}
          onChange={(e) => {
            const val = e.target.value.replace(/[^a-fA-F0-9]/g, '');
            if (val.length <= 64) setSyncKeyInput(val.toLowerCase());
          }}
          placeholder="Paste 64-character Sync Key here..."
          className="w-full bg-slate-50 dark:bg-gray-800 text-slate-700 dark:text-gray-300 px-3 py-2 rounded-lg text-xs font-mono border border-slate-200 dark:border-gray-700 mb-2 h-12 outline-none focus:ring-2 focus:ring-indigo-500 break-all selectable-text"
        />
        <div className="flex gap-2 mb-4">
          <button
            onClick={() => setSyncKeyInput("")}
            className="flex-1 py-1.5 bg-slate-100 dark:bg-gray-700 hover:bg-slate-200 dark:hover:bg-gray-600 text-slate-700 dark:text-gray-300 rounded text-xs font-medium transition-colors"
          >
            Clear
          </button>
        </div>
        <p className="text-[10px] text-slate-500 dark:text-gray-500 mb-3 text-center">
          Share this key with other devices, or paste theirs here to securely link them together.
        </p>
        <div className="flex gap-2">
          <button
            onClick={async () => {
              await writeTextToClipboard(syncKey);
              setCopiedKey(true);
              setTimeout(() => setCopiedKey(false), 2000);
            }}
            className={`flex-1 py-2 rounded-lg text-sm font-medium transition-colors border cursor-pointer ${copiedKey ? 'bg-emerald-50 text-emerald-600 border-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-400 dark:border-emerald-500/20' : 'bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 border-slate-200 dark:border-gray-700'}`}
          >
            {copiedKey ? <span className="flex items-center justify-center gap-1"><Check className="w-4 h-4" /> Copied!</span> : "Copy"}
          </button>
          <button
            onClick={updateSyncKey}
            className="flex-1 py-2 bg-indigo-500 hover:bg-indigo-600 text-white rounded-lg text-sm font-medium transition-colors shadow-md shadow-indigo-500/20 cursor-pointer"
          >
            Save Key
          </button>
        </div>
      </div>
    </div>
  );

  const displayedClips = activeTab === 'pinned' ? clips.filter(c => c.pinned) : clips.filter(c => !c.pinned);

  if (isScanning) {
    return (
      <div className="fixed inset-0 z-[9999] bg-transparent flex flex-col items-center justify-center">
        <div className="w-64 h-64 border-2 border-emerald-500 border-dashed rounded-3xl mb-8 shadow-[0_0_0_9999px_rgba(0,0,0,0.5)] flex items-center justify-center">
          <span className="text-white/80 font-medium text-sm drop-shadow-md">Align QR Code Here</span>
        </div>
        <button 
          onClick={handleCancelScan}
          className="px-6 py-3 bg-red-500 hover:bg-red-600 text-white rounded-full font-medium shadow-lg transition-transform active:scale-95"
        >
          Cancel Scan
        </button>
      </div>
    );
  }

  return (
    <div className={`min-h-screen ${isScanning ? 'bg-transparent' : 'bg-slate-50 dark:bg-[#0d1117]'} text-slate-800 dark:text-gray-200 p-6 ${isMobile ? 'pt-14' : 'pt-6'} flex flex-col items-center transition-colors`}>
      <header className="w-full max-w-xl flex justify-between items-center mb-6 pb-4 border-b border-slate-200 dark:border-gray-800 transition-all duration-300">
        <div className="flex items-center gap-3">
          <div className="bg-indigo-500/10 p-2 rounded-xl">
            <MonitorSmartphone className="text-indigo-400 w-6 h-6" />
          </div>
          <h1 className="text-2xl font-bold bg-gradient-to-r from-indigo-400 to-cyan-400 bg-clip-text text-transparent tracking-tight">
            CipherClip
          </h1>
        </div>
        <div className="flex items-center gap-2">
          <Tooltip text="Connected Devices (Ctrl+D)">
            <button 
              onClick={() => setShowConnectedDevicesModal(true)}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-semibold transition-colors border cursor-pointer ${connectedPeers.length > 0 ? 'bg-emerald-50 dark:bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-100 dark:hover:bg-emerald-500/20 border-emerald-200 dark:border-emerald-500/20' : 'bg-slate-100 dark:bg-gray-800 text-slate-500 dark:text-gray-400 hover:bg-slate-200 dark:hover:bg-gray-700 border-slate-200 dark:border-gray-700'}`}
            >
              <MonitorSmartphone className="w-4 h-4" />
              <span>{connectedPeers.length}</span>
            </button>
          </Tooltip>
          <Tooltip text="Settings (Ctrl+I)" side="bottom">
            <button 
              onClick={() => setShowSettings(true)}
              className="p-2 hover:bg-slate-200 dark:hover:bg-gray-800 text-slate-500 dark:text-gray-400 rounded-xl transition-colors"
            >
              <SlidersHorizontal className="w-5 h-5" />
            </button>
          </Tooltip>
        </div>
      </header>

      <main className="w-full max-w-xl flex-1 flex flex-col gap-3 transition-all duration-300">
        {loading ? (
          <div className="flex justify-center items-center py-20">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div>
          </div>
        ) : clips.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-20 text-slate-500 dark:text-gray-500 gap-4">
            <Copy className="w-12 h-12 opacity-30 dark:opacity-20" />
            <p className="text-slate-600 dark:text-gray-400 font-medium">Your clipboard history is empty.</p>
            <p className="text-sm text-slate-400 dark:text-gray-500">Copy something to see it appear here, securely.</p>
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            {hasPinned && (
              <div className="flex bg-slate-200/50 dark:bg-gray-800/50 p-1 rounded-xl w-full">
                <button 
                  onClick={() => setActiveTab('recent')} 
                  className={`flex-1 text-center py-2 text-sm font-medium rounded-lg transition-colors ${activeTab === 'recent' ? 'bg-white dark:bg-[#161b22] text-slate-800 dark:text-gray-200 shadow-sm' : 'text-slate-500 dark:text-gray-400 hover:text-slate-700 dark:hover:text-gray-300'}`}
                >
                  Recent
                </button>
                <button 
                  onClick={() => setActiveTab('pinned')} 
                  className={`flex-1 text-center py-2 text-sm font-medium rounded-lg transition-colors ${activeTab === 'pinned' ? 'bg-white dark:bg-[#161b22] text-slate-800 dark:text-gray-200 shadow-sm' : 'text-slate-500 dark:text-gray-400 hover:text-slate-700 dark:hover:text-gray-300'}`}
                >
                  Pinned
                </button>
              </div>
            )}
            
            <div className="flex flex-col gap-3 overflow-hidden">
              <AnimatePresence mode="wait">
                <motion.div
                  key={activeTab}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                  className="flex flex-col"
                >
                  <AnimatePresence>
                    {displayedClips.map((clip) => (
                      <ClipCard 
                        key={clip.id} 
                        clip={clip} 
                        copiedId={copiedId} 
                        hasMasterPassword={hasMasterPassword}
                        handleCopy={(c, autoPaste) => handleCopy(c, autoPaste, false)} 
                        togglePin={togglePin} 
                        deleteClip={deleteClip} 
                        requestUnlock={(clipId, action, autoPaste) => {
                          setTimeout(() => setShowPasswordPrompt({ clipId, isAutoPaste: autoPaste || false, action }), 100);
                        }}
                        toggleLock={toggleLock}
                        requestSetup={(id) => {
                          if (id !== undefined) setPendingLockId(id);
                          setTimeout(() => setShowPasswordSetup(true), 100);
                        }}
                        onPreviewImage={async (base64) => {
                          showToast("Opening image in system viewer...");
                          await invoke('open_image_preview', { base64Data: base64 });
                        }}
                      />
                    ))}
                  </AnimatePresence>
                </motion.div>
              </AnimatePresence>
            </div>
          </div>
        )}
      </main>

      {/* Alert Modal */}
      <AnimatePresence>
        {alertModal && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 bg-black/60 backdrop-blur-sm z-[70] flex items-center justify-center p-4"
            onClick={() => setAlertModal(null)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl w-full max-w-sm overflow-hidden shadow-2xl flex flex-col"
            >
              <div className={`p-4 border-b flex items-center gap-3 ${alertModal.isError ? 'border-red-100 dark:border-red-500/20 bg-red-50 dark:bg-red-500/10' : 'border-emerald-100 dark:border-emerald-500/20 bg-emerald-50 dark:bg-emerald-500/10'}`}>
                <div className={`p-2 rounded-full ${alertModal.isError ? 'bg-red-100 dark:bg-red-500/20 text-red-600 dark:text-red-400' : 'bg-emerald-100 dark:bg-emerald-500/20 text-emerald-600 dark:text-emerald-400'}`}>
                  {alertModal.isError ? <X className="w-5 h-5" /> : <Check className="w-5 h-5" />}
                </div>
                <h2 className={`font-semibold ${alertModal.isError ? 'text-red-800 dark:text-red-400' : 'text-emerald-800 dark:text-emerald-400'}`}>
                  {alertModal.isError ? "Error" : "Success"}
                </h2>
              </div>
              <div className="p-6 bg-slate-50/50 dark:bg-[#0d1117]/50">
                <p className="text-sm text-slate-600 dark:text-gray-300 mb-6 leading-relaxed">
                  {alertModal.message}
                </p>
                <button
                  onClick={() => setAlertModal(null)}
                  className={`w-full py-2.5 rounded-lg text-sm font-medium transition-colors ${
                    alertModal.isError 
                      ? 'bg-slate-200 hover:bg-slate-300 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-800 dark:text-gray-200' 
                      : 'bg-indigo-500 hover:bg-indigo-600 text-white shadow-md shadow-indigo-500/20'
                  }`}
                >
                  OK
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Encryption Details Modal */}
      <AnimatePresence>
        {showEncryptionModal && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm"
            onClick={() => setShowEncryptionModal(false)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl w-full max-w-sm overflow-hidden shadow-2xl flex flex-col"
            >
              <div className="p-5 border-b border-emerald-100 dark:border-emerald-500/20 bg-emerald-50/50 dark:bg-emerald-500/5 flex items-center gap-4">
                <div className="p-3 rounded-full bg-emerald-100 dark:bg-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                  <ShieldCheck className="w-6 h-6" />
                </div>
                <div>
                  <h2 className="font-semibold text-slate-800 dark:text-gray-100">Military-Grade Security</h2>
                  <p className="text-xs text-slate-500 dark:text-gray-400">Your data never touches the cloud</p>
                </div>
              </div>
              <div className="p-6 bg-slate-50/50 dark:bg-[#0d1117]/50 flex flex-col gap-4">
                <div className="text-sm text-slate-600 dark:text-gray-300 leading-relaxed">
                  <p className="mb-3">
                    CipherClip uses <strong>XChaCha20-Poly1305</strong> peer-to-peer encryption over your local network. 
                  </p>
                  <p>
                    When you pair a device using your Sync Key, the devices communicate directly with each other. No servers, no accounts, no cloud databases. If a payload is intercepted on your Wi-Fi, it is mathematically impossible to decrypt.
                  </p>
                </div>
                {connectedPeers.length > 0 && (
                  <div className="bg-emerald-50 dark:bg-emerald-500/10 border border-emerald-200 dark:border-emerald-500/20 rounded-xl p-4 mt-2">
                    <h4 className="text-sm font-semibold text-emerald-700 dark:text-emerald-400 mb-2 flex items-center gap-2">
                      <Network className="w-4 h-4" />
                      Connected Devices ({connectedPeers.length})
                    </h4>
                    <ul className="space-y-1">
                      {connectedPeers.map(peer => (
                        <li key={peer.ip} className="text-xs font-mono text-emerald-600 dark:text-emerald-500 flex items-center gap-2">
                          <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span>
                          <span className="truncate" title={peer.name}>{peer.name}</span>
                          <span className="opacity-70 text-[10px] ml-1">({peer.ip})</span>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
                {networkSyncUI}
                <button
                  onClick={() => setShowEncryptionModal(false)}
                  className="w-full mt-2 py-2.5 rounded-lg text-sm font-medium transition-colors bg-emerald-500 hover:bg-emerald-600 text-white shadow-md shadow-emerald-500/20"
                >
                  Got it
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Tutorial Modal */}
      <AnimatePresence>
        {showTutorial && (
          <motion.div 
            initial={{ opacity: 0 }} 
            animate={{ opacity: 1 }} 
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[100] flex items-center justify-center p-4 bg-black/60 backdrop-blur-md"
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-3xl w-full max-w-lg overflow-hidden shadow-2xl flex flex-col items-center p-8 text-center max-h-[90vh] overflow-y-auto custom-scrollbar"
            >
              <div className="w-20 h-20 bg-indigo-500/10 rounded-3xl flex items-center justify-center mb-6 border border-indigo-500/20">
                <MonitorSmartphone className="text-indigo-500 w-10 h-10" />
              </div>
              <h2 className="text-2xl font-bold text-slate-800 dark:text-gray-100 mb-3">Connected Devices</h2>
              
              {!isMobile && (
                <div className="flex flex-col mb-6 w-full">
                  <button 
                    onClick={() => setShowPairing(!showPairing)}
                    className="flex items-center justify-between p-3 bg-indigo-50 dark:bg-indigo-500/10 hover:bg-indigo-100 dark:hover:bg-indigo-500/20 text-indigo-600 dark:text-indigo-400 rounded-xl transition-colors font-medium border border-indigo-100 dark:border-indigo-500/20"
                  >
                    <div className="flex items-center gap-2">
                      <QrCode className="w-5 h-5" />
                      <span>Connect a device</span>
                    </div>
                    {showPairing ? <Maximize2 className="w-4 h-4 rotate-45" /> : <Plus className="w-4 h-4" />}
                  </button>

                  <AnimatePresence>
                    {showPairing && (
                      <motion.div 
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: 'auto' }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden"
                      >
                        <div className="flex flex-col items-center bg-slate-50 dark:bg-gray-800/50 p-4 rounded-xl w-full border border-slate-100 dark:border-gray-800 mt-2">
                          <p className="text-sm font-semibold text-slate-800 dark:text-gray-200 mb-3">Scan with Mobile App</p>
                          <div className="bg-white p-3 rounded-xl shadow-sm border border-slate-200 dark:border-gray-700 mb-4">
                            <QRCodeSVG value={syncKey} size={140} level="H" includeMargin={false} />
                          </div>
                          
                          <div className="w-full bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-700 rounded-lg p-3 text-center cursor-pointer group hover:border-indigo-400 dark:hover:border-indigo-500 transition-colors"
                            onClick={async () => {
                              try {
                                await writeTextToClipboard(syncKey);
                              } catch(e) {
                                await navigator.clipboard.writeText(syncKey);
                              }
                              showToast("Sync key copied!");
                            }}
                          >
                            <div className="text-xs text-slate-500 dark:text-gray-400 mb-1">Or copy device key</div>
                            <div className="text-sm font-mono text-slate-700 dark:text-gray-300 break-all">{syncKey}</div>
                          </div>
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>
                </div>
              )}
              
              <p className="text-slate-600 dark:text-gray-400 mb-6 max-w-sm leading-relaxed">
                The most secure, lightning-fast clipboard manager. Keep your clips synced across devices, fully E2E encrypted on your local network.
              </p>
              
              <div className="flex flex-col gap-4 w-full mb-8 text-left">
                <div className="flex gap-4 items-start bg-slate-50 dark:bg-[#0d1117]/50 p-4 rounded-2xl border border-slate-100 dark:border-gray-800/50">
                  <ShieldCheck className="w-6 h-6 text-emerald-500 mt-0.5 shrink-0" />
                  <div>
                    <h4 className="font-semibold text-slate-800 dark:text-gray-200">Local Network Sync</h4>
                    <p className="text-xs text-slate-500 dark:text-gray-400 mt-1">Connect your phone and PC by scanning a QR code. Clips sync instantly and securely over your Wi-Fi.</p>
                  </div>
                </div>
                <div className="flex gap-4 items-start bg-slate-50 dark:bg-[#0d1117]/50 p-4 rounded-2xl border border-slate-100 dark:border-gray-800/50">
                  <Key className="w-6 h-6 text-indigo-500 mt-0.5 shrink-0" />
                  <div>
                    <h4 className="font-semibold text-slate-800 dark:text-gray-200">Master Password</h4>
                    <p className="text-xs text-slate-500 dark:text-gray-400 mt-1">Set a master password to encrypt your history on-disk and lock sensitive clips.</p>
                  </div>
                </div>
              </div>

              <button
                onClick={closeTutorial}
                className="w-full py-3.5 rounded-xl text-sm font-semibold transition-all bg-indigo-500 hover:bg-indigo-600 text-white shadow-lg shadow-indigo-500/25 active:scale-[0.98]"
              >
                Let's Get Started
              </button>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>


      <AnimatePresence>
        {showConnectedDevicesModal && (
          <motion.div 
            initial={{ opacity: 0 }} 
            animate={{ opacity: 1 }} 
            exit={{ opacity: 0 }} 
            className="fixed inset-0 bg-black/40 backdrop-blur-sm z-[70] flex items-center justify-center p-4" 
            onClick={() => setShowConnectedDevicesModal(false)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }} 
              animate={{ scale: 1, opacity: 1, y: 0 }} 
              exit={{ scale: 0.95, opacity: 0, y: 10 }} 
              onClick={(e) => e.stopPropagation()} 
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl w-full max-w-sm overflow-hidden shadow-2xl flex flex-col"
            >
              <div className="p-4 border-b border-slate-100 dark:border-gray-800 flex justify-between items-center bg-slate-50 dark:bg-[#1e242c]">
                <div className="flex items-center gap-2">
                  <Network className="w-5 h-5 text-indigo-500" />
                  <h3 className="font-semibold text-slate-800 dark:text-gray-200">Connected Devices</h3>
                </div>
                <div className="flex items-center gap-2">
                  {isMobile && (
                    <button
                      onClick={handleScanQR}
                      className="flex items-center gap-1.5 px-2.5 py-1 bg-indigo-50 dark:bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 hover:bg-indigo-100 dark:hover:bg-indigo-500/20 rounded-md text-[11px] font-medium transition-colors cursor-pointer"
                    >
                      <Scan className="w-3.5 h-3.5" />
                      Scan QR
                    </button>
                  )}
                  <button onClick={() => setShowConnectedDevicesModal(false)} className="p-1 text-slate-400 hover:text-slate-600 dark:text-gray-500 dark:hover:text-gray-300">
                    <X className="w-5 h-5" />
                  </button>
                </div>
              </div>
              <div className="p-4 max-h-[60vh] overflow-y-auto">
                {connectedPeers.length === 0 ? (
                  <div className="text-center p-6">
                    <MonitorSmartphone className="w-12 h-12 text-slate-300 dark:text-gray-700 mx-auto mb-3" />
                    <p className="text-slate-500 dark:text-gray-400 text-sm">No devices connected on the local network.</p>
                  </div>
                ) : (
                  <div className="flex flex-col gap-2">
                    {connectedPeers.map(peer => (
                      <div key={peer.ip} className="flex items-center justify-between p-3 bg-slate-50 dark:bg-gray-800/50 rounded-xl border border-slate-100 dark:border-gray-800 group">
                        <div className="flex items-center gap-3">
                          <div className="w-2 h-2 rounded-full bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)] animate-pulse"></div>
                          <div className="flex flex-col">
                            <span className="font-medium text-slate-800 dark:text-gray-200 text-sm">{peer.name}</span>
                            <span className="text-slate-500 dark:text-gray-400 font-mono text-xs">{peer.ip}</span>
                          </div>
                        </div>
                        <button 
                          onClick={async () => {
                            try {
                              await invoke("disconnect_peer", { ip: peer.ip });
                              setConnectedPeers(prev => prev.filter(p => p.ip !== peer.ip));
                            } catch(e) {}
                          }}
                          className="p-1.5 text-slate-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10 rounded-lg transition-colors md:opacity-0 md:group-hover:opacity-100 opacity-100"
                          title="Disconnect"
                        >
                          <X className="w-4 h-4" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Settings Modal */}
      <AnimatePresence>
        {showSettings && (
          <motion.div 
            initial={{ opacity: 0 }} 
            animate={{ opacity: 1 }} 
            exit={{ opacity: 0 }} 
            className="fixed inset-0 bg-slate-900/40 backdrop-blur-sm z-50 flex items-center justify-center p-6"
            onClick={() => {
              setShowSettings(false);
              setShortcutInput(shortcut); // Reset input if closed without saving
              setShowRecycleBin(false);
              setShowNetworkSync(false);
            }}
          >
            <motion.div 
              initial={{ scale: 0.95, y: 10 }} 
              animate={{ scale: 1, y: 0 }} 
              exit={{ scale: 0.95, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl w-full max-w-sm overflow-hidden shadow-2xl flex flex-col max-h-[85vh]"
            >
              {showNetworkSync ? (
                <>
                  <div className="p-4 border-b border-slate-100 dark:border-gray-800 flex justify-between items-center bg-slate-50 dark:bg-[#1e242c]">
                    <div className="flex items-center gap-2">
                      <button 
                        onClick={() => setShowNetworkSync(false)}
                        className="p-1.5 text-slate-400 hover:text-slate-600 dark:text-gray-500 dark:hover:text-gray-300 bg-white dark:bg-[#161b22] rounded-md shadow-sm border border-slate-200 dark:border-gray-700"
                      >
                        <ArrowLeft className="w-4 h-4" />
                      </button>
                      <h2 className="font-semibold text-slate-800 dark:text-gray-200 flex items-center gap-2">
                        <Network className="w-4 h-4 text-indigo-500" /> Network Sync
                      </h2>
                    </div>
                  </div>
                  <div className="p-6 overflow-y-auto custom-scrollbar flex-1 bg-slate-50/50 dark:bg-[#0d1117]/50 flex flex-col gap-4">
                    {networkSyncUI}
                    {connectedPeers.length > 0 && (
                      <div className="bg-emerald-50 dark:bg-emerald-500/10 border border-emerald-200 dark:border-emerald-500/20 rounded-xl p-4">
                        <h4 className="text-sm font-semibold text-emerald-700 dark:text-emerald-400 mb-2 flex items-center gap-2">
                          <Network className="w-4 h-4" />
                          Connected Devices ({connectedPeers.length})
                        </h4>
                        <ul className="space-y-1">
                          {connectedPeers.map(peer => (
                            <li key={peer.ip} className="text-xs font-mono text-emerald-600 dark:text-emerald-500 flex items-center gap-2">
                              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span>
                              <span className="truncate" title={peer.name}>{peer.name}</span>
                              <span className="opacity-70 text-[10px] ml-1">({peer.ip})</span>
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                    <p className="text-xs text-slate-500 dark:text-gray-500 leading-relaxed text-center mt-2 px-2">
                      Scan the QR code with your other device or manually copy and paste the exact same <strong>Sync Key</strong> to pair them. Once paired on the same <strong>Wi-Fi or Mobile Hotspot</strong>, your clipboards will sync securely and automatically without any issues.
                    </p>
                  </div>
                </>
              ) : showRecycleBin ? (
                <>
                  <div className="p-4 border-b border-slate-100 dark:border-gray-800 flex justify-between items-center bg-slate-50 dark:bg-[#1e242c]">
                    <div className="flex items-center gap-2">
                      <button 
                        onClick={() => setShowRecycleBin(false)}
                        className="p-1.5 text-slate-400 hover:text-slate-600 dark:text-gray-500 dark:hover:text-gray-300 bg-white dark:bg-[#161b22] rounded-md shadow-sm border border-slate-200 dark:border-gray-700"
                      >
                        <ArrowLeft className="w-4 h-4" />
                      </button>
                      <h2 className="font-semibold text-slate-800 dark:text-gray-200 flex items-center gap-2">
                        <Trash2 className="w-4 h-4" /> Recycle Bin
                      </h2>
                    </div>
                    {deletedClips.length > 0 && (
                      <button 
                        onClick={handleEmptyRecycleBin}
                        className="text-xs font-medium text-red-500 hover:text-red-600 bg-red-50 dark:bg-red-500/10 px-2 py-1 rounded transition-colors"
                      >
                        Empty All
                      </button>
                    )}
                  </div>
                  <div className="p-4 overflow-y-auto custom-scrollbar flex-1 bg-slate-50/50 dark:bg-[#0d1117]/50">
                    <p className="text-xs text-slate-600 dark:text-gray-400 mb-4 bg-indigo-50 dark:bg-indigo-500/10 p-3 rounded-lg border border-indigo-100 dark:border-indigo-900/30 flex gap-2 items-start">
                      <AlertTriangle className="w-4 h-4 text-indigo-500 shrink-0 mt-0.5" />
                      <span>Items automatically deleted when reaching your history limit bypass the recycle bin and are permanently removed to save space.</span>
                    </p>
                    {deletedClips.length === 0 ? (
                      <div className="text-center py-8 text-slate-400 dark:text-gray-600 text-sm flex flex-col items-center gap-2">
                        <RefreshCcw className="w-8 h-8 opacity-20" />
                        Recycle bin is empty
                      </div>
                    ) : (
                      <div className="flex flex-col gap-3">
                        {deletedClips.map(clip => (
                          <div key={clip.id} className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-lg p-3 shadow-sm">
                            <div className="text-xs text-slate-600 dark:text-gray-300 mb-2 truncate font-mono selectable-text">
                              {clip.content_type === "image" ? "[Image]" : clip.content.substring(0, 150)}
                            </div>
                            <div className="flex justify-between items-center">
                              <span className="text-[10px] text-slate-400 dark:text-gray-500 flex items-center gap-1">
                                <Clock className="w-3 h-3" />
                                {new Date(clip.timestamp * 1000).toLocaleDateString()}
                              </span>
                              <div className="flex gap-1">
                                <button onClick={() => restoreClip(clip.id)} className="p-1.5 text-emerald-500 hover:bg-emerald-50 dark:hover:bg-emerald-500/10 rounded transition-colors" title="Restore">
                                  <RefreshCcw className="w-4 h-4" />
                                </button>
                                <button onClick={() => permanentlyDeleteClip(clip.id)} className="p-1.5 text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10 rounded transition-colors" title="Delete Forever">
                                  <Trash2 className="w-4 h-4" />
                                </button>
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </>
              ) : (
                <>
                  <div className="px-5 pt-5 pb-2 flex justify-between items-center">
                    <h2 className="text-xl font-bold text-slate-800 dark:text-gray-200">Settings</h2>
                    <button 
                      onClick={() => {
                        setShowSettings(false);
                        setShortcutInput(shortcut);
                        setShowRecycleBin(false);
                        setShowNetworkSync(false);
                      }} 
                      className="p-1 text-slate-400 hover:text-slate-600 dark:text-gray-500 dark:hover:text-gray-300"
                    >
                      <X className="w-5 h-5" />
                    </button>
                  </div>
                  
                  {/* Settings Navigation Tabs */}
                  <div className="flex w-full border-b border-slate-100 dark:border-gray-800 px-3 pt-2 gap-1">
                    {(['general', 'sync', 'data', 'about'] as const).map(tab => (
                      <button
                        key={tab}
                        onClick={() => setSettingsTab(tab)}
                        className={`flex-1 text-center pb-3 text-sm font-medium capitalize transition-colors border-b-2 ${settingsTab === tab ? 'text-indigo-500 border-indigo-500' : 'text-slate-500 dark:text-gray-400 border-transparent hover:text-slate-700 dark:hover:text-gray-300'}`}
                      >
                        {tab}
                      </button>
                    ))}
                  </div>

                  <div className="p-6 flex flex-col gap-6 overflow-y-auto custom-scrollbar">
                    {settingsTab === 'general' && (
                      <>
                        {/* Theme */}
                        <div>
                          <label className="text-sm font-medium text-slate-700 dark:text-gray-300 mb-2 block">Theme</label>
                          <div className="grid grid-cols-3 gap-2 mb-2">
                            {['light', 'dark', 'system'].map(t => (
                              <button
                                key={t}
                                onClick={() => updateTheme(t)}
                                className={`py-2 text-sm font-medium rounded-lg capitalize transition-colors ${
                                  theme === t 
                                  ? 'bg-indigo-500 text-white shadow-md shadow-indigo-500/20' 
                                  : 'bg-slate-100 dark:bg-gray-800 text-slate-600 dark:text-gray-400 hover:bg-slate-200 dark:hover:bg-gray-700'
                                }`}
                              >
                                {t}
                              </button>
                            ))}
                          </div>
                        </div>

                        {/* Master Password */}
                        <div>
                          <div className="flex items-center justify-between mb-2">
                            <label className="flex items-center gap-1.5 text-sm font-medium text-slate-700 dark:text-gray-300"><Key className="w-4 h-4 text-slate-400" /> Master Password</label>
                            {hasMasterPassword ? (
                              <button
                                onClick={() => setShowPasswordSetup(true)}
                                className="px-3 py-1 bg-red-50 hover:bg-red-100 dark:bg-red-500/10 dark:hover:bg-red-500/20 text-red-600 dark:text-red-400 rounded-lg text-xs font-medium transition-colors"
                              >
                                Remove Password
                              </button>
                            ) : (
                              <button
                                onClick={() => setShowPasswordSetup(true)}
                                className="px-3 py-1 bg-indigo-50 hover:bg-indigo-100 dark:bg-indigo-500/10 dark:hover:bg-indigo-500/20 text-indigo-600 dark:text-indigo-400 rounded-lg text-xs font-medium transition-colors"
                              >
                                Set Password
                              </button>
                            )}
                          </div>
                          <p className="text-xs text-slate-500 dark:text-gray-500">Lock your sensitive clips manually. Requires your master password to unlock and copy.</p>
                        </div>

                        <div>
                          <label className="text-sm font-medium text-slate-700 dark:text-gray-300 mb-2 block">Clipboard History Limit</label>
                          <p className="text-xs text-slate-500 dark:text-gray-500 mb-3">Auto-delete oldest unpinned clips when limit is reached.</p>
                          <div className="grid grid-cols-4 gap-2">
                            {[50, 100, 200, 500].map(val => (
                              <button
                                key={val}
                                onClick={() => updateLimit(val)}
                                className={`py-2 text-sm font-medium rounded-lg transition-colors ${
                                  limit === val 
                                  ? 'bg-indigo-500 text-white shadow-md shadow-indigo-500/20' 
                                  : 'bg-slate-100 dark:bg-gray-800 text-slate-600 dark:text-gray-400 hover:bg-slate-200 dark:hover:bg-gray-700'
                                }`}
                              >
                                {val}
                              </button>
                            ))}
                          </div>
                          <AnimatePresence>
                            {limitSaved && (
                              <motion.p 
                                initial={{ opacity: 0, y: -5 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0 }}
                                className="text-xs text-emerald-500 dark:text-emerald-400 mt-2 text-center font-medium"
                              >
                                Limit updated successfully!
                              </motion.p>
                            )}
                          </AnimatePresence>
                        </div>

                        { !isMobile && (
                          <>
                            <div>
                              <div className="flex items-center justify-between mb-2">
                                <label className="text-sm font-medium text-slate-700 dark:text-gray-300">Auto-Start on Boot</label>
                                <button
                                  onClick={toggleAutoStart}
                                  className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${autoStart ? 'bg-indigo-500' : 'bg-slate-300 dark:bg-gray-600'}`}
                                >
                                  <span className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${autoStart ? 'translate-x-4.5' : 'translate-x-1'}`} />
                                </button>
                              </div>
                              <p className="text-xs text-slate-500 dark:text-gray-500">Launch CipherClip quietly in the background when your computer starts.</p>
                            </div>

                            <div>
                              <label className="text-sm font-medium text-slate-700 dark:text-gray-300 mb-2 block">Global Shortcut</label>
                              <p className="text-xs text-slate-500 dark:text-gray-500 mb-3">Click the box and press any key combination. Press <strong>Backspace</strong> to clear.</p>
                              <div className="flex gap-2">
                                <input 
                                  type="text" 
                                  value={shortcutInput}
                                  onChange={() => {}} // Controlled by onKeyDown
                                  onKeyDown={handleShortcutKeyDown}
                                  onBlur={() => {
                                    if (!shortcutInput.trim()) setShortcutInput(shortcut);
                                  }}
                                  placeholder="Press keys..."
                                  className="flex-1 bg-slate-100 dark:bg-gray-800 text-slate-700 dark:text-gray-300 px-3 py-2 rounded-lg text-sm font-mono border border-slate-200 dark:border-gray-700 focus:ring-2 focus:ring-indigo-500 outline-none"
                                />
                                <button
                                  onClick={updateShortcut}
                                  className={`px-3 py-2 rounded-lg text-sm font-medium transition-colors flex items-center gap-1 ${
                                    shortcutSaved ? 'bg-emerald-500 hover:bg-emerald-600 text-white' : 'bg-indigo-500 hover:bg-indigo-600 text-white'
                                  }`}
                                >
                                  {shortcutSaved ? <><Check className="w-4 h-4" /> Saved!</> : "Save"}
                                </button>
                              </div>
                            </div>
                          </>
                        )}
                      </>
                    )}

                    {settingsTab === 'sync' && (
                      <div className="flex flex-col gap-4">
                        <div>
                          <label className="text-sm font-medium text-slate-700 dark:text-gray-300 mb-2 block">Local Network Sync</label>
                          <p className="text-xs text-slate-500 dark:text-gray-500 mb-3">Sync clips between your devices over Wi-Fi without any cloud servers using End-to-End Encryption.</p>
                          {!isMobile && (
                            <button
                              onClick={() => {
                                fetchSyncKey();
                                setShowNetworkSync(true);
                              }}
                              className="w-full py-2.5 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-indigo-600 dark:text-indigo-400 rounded-lg text-sm font-medium transition-colors border border-slate-200 dark:border-gray-700 flex items-center justify-center gap-2"
                            >
                              <Network className="w-4 h-4" />
                              Configure Network Sync
                            </button>
                          )}
                        </div>
                        {isMobile && networkSyncUI}
                      </div>
                    )}

                    {settingsTab === 'data' && (
                      <div>
                        <label className="text-sm font-medium text-slate-700 dark:text-gray-300 mb-2 block">Data Management</label>
                        <p className="text-xs text-slate-500 dark:text-gray-500 mb-3">Manage deleted clips or completely wipe your history.</p>
                        
                        <div className="flex flex-col gap-2">
                          <button
                            onClick={() => {
                              fetchDeletedHistory();
                              setShowRecycleBin(true);
                            }}
                            className="w-full py-2.5 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-lg text-sm font-medium transition-colors border border-slate-200 dark:border-gray-700 flex items-center justify-center gap-2"
                          >
                            <Trash2 className="w-4 h-4" />
                            Recycle Bin
                          </button>

                          <button
                            onClick={() => setShowConfirmClear(true)}
                            className="w-full py-2.5 bg-red-50 hover:bg-red-100 dark:bg-red-500/10 dark:hover:bg-red-500/20 text-red-600 dark:text-red-400 rounded-lg text-sm font-medium transition-colors border border-red-200 dark:border-red-500/30"
                          >
                            Clear All History
                          </button>
                        </div>
                      </div>
                    )}

                    {settingsTab === 'about' && (
                      <div className="flex flex-col items-center pb-2 text-center">
                        <div className="w-16 h-16 bg-indigo-500/10 rounded-2xl flex items-center justify-center mb-3 border border-indigo-500/20">
                          <MonitorSmartphone className="text-indigo-500 w-8 h-8" />
                        </div>
                        <h3 className="font-bold text-lg text-slate-800 dark:text-gray-200">CipherClip</h3>
                        <p className="text-xs text-slate-500 dark:text-gray-500 font-mono mb-3">Version 0.1.0</p>
                        
                        <p className="text-sm text-slate-600 dark:text-gray-400 max-w-xs leading-relaxed mb-4">
                          A blazingly fast, mathematically secure, peer-to-peer clipboard manager built with Rust, Tauri, and React.
                        </p>
                        
                        <div className="w-full bg-slate-50 dark:bg-gray-800/50 rounded-xl p-3 border border-slate-200 dark:border-gray-800/50 text-left mb-3">
                          <a href="https://github.com/TechyNisarg/CipherClip" target="_blank" rel="noreferrer" className="flex items-center justify-center gap-2 text-sm text-slate-700 dark:text-gray-300 hover:text-indigo-600 dark:hover:text-indigo-400 transition-colors">
                            <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-4 h-4"><path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4"></path><path d="M9 18c-4.51 2-5-2-7-2"></path></svg>
                            <span className="font-medium">https://github.com/TechyNisarg/CipherClip</span>
                          </a>
                        </div>
                        
                        <div className="w-full bg-slate-50 dark:bg-gray-800/50 rounded-xl p-3 border border-slate-200 dark:border-gray-800/50 text-left">
                          <p className="text-xs text-slate-500 dark:text-gray-500 mb-1">Architecture</p>
                          <ul className="text-xs font-mono text-slate-600 dark:text-gray-400 space-y-1">
                            <li>• Tauri v2 / Rust Backend</li>
                            <li>• React + Tailwind CSS v4</li>
                            <li>• XChaCha20-Poly1305 E2EE</li>
                            <li>• Local SQLite Storage</li>
                          </ul>
                        </div>
                        
                        <p className="text-[10px] text-slate-400 dark:text-gray-600 mt-4 uppercase tracking-widest font-semibold">
                          Open Source • Private • Fast
                        </p>
                      </div>
                    )}
                  </div>
                </>
              )}
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Confirmation Modal */}
      <AnimatePresence>
        {showConfirmClear && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[60] flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm"
            onClick={() => setShowConfirmClear(false)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl p-6 w-full max-w-sm shadow-2xl"
            >
              <div className="flex items-center gap-4 mb-4">
                <div className="w-10 h-10 rounded-full bg-red-100 dark:bg-red-500/20 flex items-center justify-center shrink-0">
                  <AlertTriangle className="w-5 h-5 text-red-600 dark:text-red-400" />
                </div>
                <h2 className="text-lg font-semibold text-slate-800 dark:text-gray-100">Clear All History?</h2>
              </div>
              <p className="text-sm text-slate-600 dark:text-gray-400 mb-4">
                This will permanently delete all your clipboard history, including <strong>pinned items</strong>, from this device. This action cannot be undone.
              </p>
              <label className="flex items-center gap-2 text-sm text-slate-700 dark:text-gray-300 cursor-pointer mb-6 p-2 rounded-lg hover:bg-slate-50 dark:hover:bg-[#1c222b] transition-colors select-none">
                <input 
                  type="checkbox" 
                  checked={deleteLocked} 
                  onChange={(e) => setDeleteLocked(e.target.checked)}
                  className="w-4 h-4 text-red-500 rounded border-gray-300 focus:ring-red-500 dark:focus:ring-red-600 dark:ring-offset-gray-800 focus:ring-2 dark:bg-gray-700 dark:border-gray-600"
                />
                Delete locked clips as well?
              </label>
              <div className="flex gap-3">
                <button
                  onClick={() => setShowConfirmClear(false)}
                  className="flex-1 px-4 py-2 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-xl text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={executeClearHistory}
                  className="flex-1 px-4 py-2 bg-red-500 hover:bg-red-600 text-white rounded-xl text-sm font-medium transition-colors shadow-sm shadow-red-500/20"
                >
                  Yes, Delete It All
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Confirm Empty Recycle Bin Modal */}
      <AnimatePresence>
        {showConfirmEmpty && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[60] flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm"
            onClick={() => setShowConfirmEmpty(false)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl p-6 w-full max-w-sm shadow-2xl"
            >
              <div className="flex items-center gap-4 mb-4">
                <div className="w-10 h-10 rounded-full bg-red-100 dark:bg-red-500/20 flex items-center justify-center shrink-0">
                  <AlertTriangle className="w-5 h-5 text-red-600 dark:text-red-400" />
                </div>
                <h2 className="text-lg font-semibold text-slate-800 dark:text-gray-100">Empty Recycle Bin?</h2>
              </div>
              <p className="text-sm text-slate-600 dark:text-gray-400 mb-6">
                This will permanently delete all items in the recycle bin. This action cannot be undone.
              </p>
              <div className="flex gap-3">
                <button
                  onClick={() => setShowConfirmEmpty(false)}
                  className="flex-1 px-4 py-2 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-xl text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={executeEmptyRecycleBin}
                  className="flex-1 px-4 py-2 bg-red-500 hover:bg-red-600 text-white rounded-xl text-sm font-medium transition-colors shadow-sm shadow-red-500/20"
                >
                  Yes, Empty It
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Confirm Delete Locked Clip Modal */}
      <AnimatePresence>
        {clipToDelete && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[60] flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm"
            onClick={() => setClipToDelete(null)}
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl p-6 w-full max-w-sm shadow-2xl"
            >
              <div className="flex items-center gap-4 mb-4">
                <div className="w-10 h-10 rounded-full bg-red-100 dark:bg-red-500/20 flex items-center justify-center shrink-0">
                  <ShieldCheck className="w-5 h-5 text-red-600 dark:text-red-400" />
                </div>
                <h2 className="text-lg font-semibold text-slate-800 dark:text-gray-100">Delete Locked Item?</h2>
              </div>
              <p className="text-sm text-slate-600 dark:text-gray-400 mb-6">
                This item is locked. Are you sure you want to move it to the recycle bin?
              </p>
              <div className="flex gap-3">
                <button
                  onClick={() => setClipToDelete(null)}
                  className="flex-1 px-4 py-2 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-xl text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={() => executeDelete(clipToDelete.id)}
                  className="flex-1 px-4 py-2 bg-red-500 hover:bg-red-600 text-white rounded-xl text-sm font-medium transition-colors shadow-sm shadow-red-500/20"
                >
                  Yes, Delete
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Toast Notification */}
      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 20, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.95 }}
            transition={{ duration: 0.4, ease: "easeOut" }}
            className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-slate-800 dark:bg-gray-100 text-white dark:text-slate-800 px-4 py-3 rounded-xl shadow-lg shadow-slate-800/20 text-sm font-medium flex items-center justify-center gap-2.5 w-max max-w-[calc(100%-2rem)]"
          >
            <Check className="w-4 h-4 shrink-0 text-emerald-400 dark:text-emerald-600" />
            <span className="leading-snug text-left truncate">{toast}</span>
          </motion.div>
        )}
      </AnimatePresence>
      {/* Password Setup Modal */}
      <AnimatePresence>
        {showPasswordSetup && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className={`fixed inset-0 z-[60] flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm`}
            
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl p-6 w-full max-w-sm shadow-2xl"
            >
              <div className="flex items-center gap-4 mb-4">
                <div className="w-10 h-10 rounded-full bg-indigo-100 dark:bg-indigo-500/20 flex items-center justify-center shrink-0">
                  <Key className="w-5 h-5 text-indigo-600 dark:text-indigo-400" />
                </div>
                <h2 className="text-lg font-semibold text-slate-800 dark:text-gray-100">{hasMasterPassword ? "Remove Master Password" : "Set Master Password"}</h2>
              </div>
              <p className="text-sm text-slate-600 dark:text-gray-400 mb-4">
                {hasMasterPassword 
                  ? "Enter your current master password to remove it. All locked clips will be unlocked."
                  : "Requires: Min 8 chars, 1 uppercase, 1 lowercase, 1 number, 1 special character."}
              </p>
              <input
                type="password"
                value={passwordInput}
                onChange={(e) => setPasswordInput(e.target.value)}
                placeholder={hasMasterPassword ? "Current Master Password" : "Enter a strong password"}
                autoFocus
                onKeyDown={(e) => e.key === 'Enter' && executeSetMasterPassword()}
                className="w-full mb-6 bg-slate-100 dark:bg-gray-800 text-slate-800 dark:text-gray-200 px-4 py-2.5 rounded-xl text-sm border border-slate-200 dark:border-gray-700 focus:ring-2 focus:ring-indigo-500 outline-none"
              />
              <div className="flex gap-3">
                <button
                  onClick={() => {
                    setShowPasswordSetup(false);
                    setPasswordInput("");
                  }}
                  className="flex-1 px-4 py-2 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-xl text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={executeSetMasterPassword}
                  className={`flex-1 px-4 py-2 text-white rounded-xl text-sm font-medium transition-colors ${hasMasterPassword ? 'bg-red-500 hover:bg-red-600' : 'bg-indigo-500 hover:bg-indigo-600'}`}
                >
                  {hasMasterPassword ? "Remove" : "Set Password"}
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>



      {/* Unlock Password Modal */}
      <AnimatePresence>
        {showPasswordPrompt && (
          <motion.div 
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className={`fixed inset-0 z-[60] flex items-center justify-center p-4 bg-black/40 backdrop-blur-sm`}
            
          >
            <motion.div 
              initial={{ scale: 0.95, opacity: 0, y: 10 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.95, opacity: 0, y: 10 }}
              onClick={(e) => e.stopPropagation()}
              className="bg-white dark:bg-[#161b22] border border-slate-200 dark:border-gray-800 rounded-2xl p-6 w-full max-w-sm shadow-2xl"
            >
              <div className="flex items-center gap-4 mb-4">
                <div className="w-10 h-10 rounded-full bg-amber-100 dark:bg-amber-500/20 flex items-center justify-center shrink-0">
                  <ShieldCheck className="w-5 h-5 text-amber-600 dark:text-amber-400" />
                </div>
                <h2 className="text-lg font-semibold text-slate-800 dark:text-gray-100">Unlock Clip</h2>
              </div>
              <p className="text-sm text-slate-600 dark:text-gray-400 mb-4">
                This clip contains sensitive information. Please enter your master password to unlock and copy it.
              </p>
              <input
                type="password"
                value={passwordInput}
                autoFocus
                onChange={(e) => setPasswordInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleUnlockClip();
                }}
                className="w-full bg-slate-50 dark:bg-gray-800 text-slate-700 dark:text-gray-300 px-4 py-3 rounded-xl text-sm border border-slate-200 dark:border-gray-700 focus:ring-2 focus:ring-indigo-500 outline-none mb-6"
                placeholder="Enter master password"
              />
              <div className="flex gap-3">
                <button
                  onClick={() => setShowPasswordPrompt(null)}
                  className="flex-1 px-4 py-2 bg-slate-100 hover:bg-slate-200 dark:bg-gray-800 dark:hover:bg-gray-700 text-slate-700 dark:text-gray-300 rounded-xl text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleUnlockClip}
                  className="flex-1 px-4 py-2 bg-indigo-500 hover:bg-indigo-600 text-white rounded-xl text-sm font-medium transition-colors"
                >
                  Unlock
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export default App;

function Tooltip({ children, text, side = "top" }: { children: React.ReactNode, text: string, side?: "top" | "bottom" | "left" | "right" }) {
  const positioning = {
    top: "bottom-[calc(100%+6px)] left-1/2 -translate-x-1/2",
    bottom: "top-[calc(100%+6px)] left-1/2 -translate-x-1/2",
    left: "right-[calc(100%+6px)] top-1/2 -translate-y-1/2",
    right: "left-[calc(100%+6px)] top-1/2 -translate-y-1/2",
  };
  const arrowPositioning = {
    top: "top-full left-1/2 -translate-x-1/2 border-t-slate-800 dark:border-t-gray-100 border-b-0",
    bottom: "bottom-full left-1/2 -translate-x-1/2 border-b-slate-800 dark:border-b-gray-100 border-t-0",
    left: "left-full top-1/2 -translate-y-1/2 border-l-slate-800 dark:border-l-gray-100 border-r-0",
    right: "right-full top-1/2 -translate-y-1/2 border-r-slate-800 dark:border-r-gray-100 border-l-0",
  };

  return (
    <div className="relative group/tooltip flex items-center justify-center">
      {children}
      <div className={`absolute ${positioning[side]} opacity-0 group-hover/tooltip:opacity-100 transition-opacity duration-200 delay-0 group-hover/tooltip:delay-[3000ms] pointer-events-none bg-slate-800 dark:bg-gray-100 text-white dark:text-slate-800 text-[11px] py-1 px-2 rounded-md whitespace-nowrap shadow-xl z-[100] font-medium`}>
        {text}
        <div className={`absolute w-0 h-0 border-[4px] border-transparent ${arrowPositioning[side]}`}></div>
      </div>
    </div>
  );
}

function ClipCard({ clip, copiedId, hasMasterPassword, handleCopy, togglePin, deleteClip, requestUnlock, toggleLock, requestSetup, onPreviewImage }: { 
  clip: ClipItem, 
  copiedId: number | null, 
  hasMasterPassword: boolean,
  handleCopy: (c: ClipItem, autoPaste?: boolean) => void, 
  togglePin: (id: number, pinned: boolean) => void, 
  deleteClip: (clip: ClipItem) => void,
  requestUnlock: (id: number, action: 'copy' | 'unlock', autoPaste?: boolean) => void,
  toggleLock: (id: number, locked: boolean) => void,
  requestSetup: (id?: number) => void,
  onPreviewImage: (base64: string) => Promise<void>
}) {

  const isLocked = clip.is_locked;
  const [isLoadingPreview, setIsLoadingPreview] = useState(false);

  return (
    <motion.div
      layout
      key={clip.id}
      onMouseDown={(e) => {
        if (isLocked) {
          e.preventDefault(); // Prevent text selection
        }
      }}
      onDoubleClick={() => {
        if (isLocked) {
          requestUnlock(clip.id, 'copy', true);
        } else {
          handleCopy(clip, true);
        }
      }}
      initial={{ opacity: 0, y: -10, scale: 0.98, marginBottom: 0 }}
      animate={{ opacity: 1, y: 0, scale: 1, marginBottom: 12 }}
      exit={{ opacity: 0, scale: 0.95, height: 0, marginTop: 0, marginBottom: 0, paddingTop: 0, paddingBottom: 0, overflow: "hidden", transition: { duration: 0.35, ease: "easeOut" } }}
      transition={{ layout: { type: "spring", stiffness: 300, damping: 25 }, opacity: { duration: 0.2 }, scale: { duration: 0.2 }, y: { duration: 0.2 } }}
      className="w-full group bg-white dark:bg-[#161b22] border border-slate-200/75 dark:border-gray-800/80 rounded-xl p-3 hover:bg-slate-50/50 dark:hover:bg-[#1c222b] hover:border-indigo-400/60 dark:hover:border-indigo-500/40 transition-colors cursor-pointer select-none overflow-hidden"
    >
      <div className="flex justify-between items-start gap-4">
        <div className="flex-1 overflow-hidden">
          {isLocked ? (
            <div 
              onClick={() => requestUnlock(clip.id, 'copy', false)}
              className="flex items-center justify-center gap-2 py-4 bg-slate-100 dark:bg-gray-800 rounded-lg border border-dashed border-slate-300 dark:border-gray-700 cursor-pointer hover:bg-slate-200 dark:hover:bg-gray-700 transition-colors"
            >
              <Key className="w-4 h-4 text-slate-400 dark:text-gray-500" />
              <span className="text-sm font-medium text-slate-500 dark:text-gray-400 select-none">Locked Content</span>
            </div>
          ) : clip.content_type === "image" ? (
            <Tooltip text="Double-click to paste image">
              <div 
                className="relative rounded-lg overflow-hidden border border-slate-200 dark:border-gray-800 bg-slate-100 dark:bg-[#0d1117] max-h-48 flex items-center justify-center group/img"
              >
                <img 
                  src={`data:image/webp;base64,${clip.content}`} 
                  alt="Copied image" 
                  draggable={true}
                  className="max-h-48 object-contain transition-transform group-hover/img:scale-[1.02] cursor-grab active:cursor-grabbing"
                />
              </div>
            </Tooltip>
          ) : (
            <p className="text-sm font-mono whitespace-pre-wrap break-words max-h-32 overflow-y-auto custom-scrollbar text-slate-700 dark:text-gray-300 selectable-text">
              {clip.content.trim()}
            </p>
          )}
        </div>
      </div>
      <div className="mt-3 flex justify-between items-center">
        <div className="flex items-center gap-2 text-xs text-slate-400 dark:text-gray-500">
          <Clock className="w-3 h-3" />
          <span>
            {new Date(clip.timestamp * 1000).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </span>
          <span className="mx-1">•</span>
          <span className="uppercase">{clip.content_type}</span>
        </div>
        <div className={`flex items-center gap-1 transition-opacity ${isMobile ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}>
          {clip.content_type === "image" && (
            <Tooltip text="Preview full image">
              <button
                disabled={isLoadingPreview}
                onClick={async (e) => {
                  e.stopPropagation();
                  if (clip.is_locked && isLocked) {
                    requestUnlock(clip.id, 'unlock', false);
                    return;
                  }
                  setIsLoadingPreview(true);
                  try {
                    await onPreviewImage(clip.content);
                  } finally {
                    setIsLoadingPreview(false);
                  }
                }}
                className={`${isMobile ? 'p-3' : 'p-1.5'} rounded-lg transition-colors cursor-pointer ${isLoadingPreview ? 'text-indigo-500 bg-indigo-50 dark:bg-indigo-500/10' : 'text-slate-400 hover:text-slate-600 hover:bg-slate-100 dark:text-gray-400 dark:hover:text-gray-200 dark:hover:bg-gray-800'}`}
              >
                {isLoadingPreview ? <Loader2 className="w-4 h-4 animate-spin" /> : <Maximize2 className="w-4 h-4" />}
              </button>
            </Tooltip>
          )}
          <Tooltip text={clip.pinned ? "Unpin item" : "Pin item"}>
            <button
              onClick={() => togglePin(clip.id, clip.pinned)}
              className={`${isMobile ? 'p-3' : 'p-1.5'} rounded-lg transition-colors cursor-pointer ${
                clip.pinned 
                ? 'text-yellow-500 bg-yellow-50 dark:bg-yellow-500/10' 
                : 'text-slate-400 hover:text-yellow-500 hover:bg-yellow-50 dark:text-gray-400 dark:hover:text-yellow-400 dark:hover:bg-yellow-500/10'
              }`}
            >
              <Pin className="w-4 h-4" fill={clip.pinned ? "currentColor" : "none"} />
            </button>
          </Tooltip>
          <Tooltip text={clip.is_locked ? "Unlock clip" : "Lock clip"}>
            <button
              onClick={() => {
                if (!hasMasterPassword) {
                  requestSetup(clip.id);
                } else if (clip.is_locked) {
                  requestUnlock(clip.id, 'unlock', false);
                } else {
                  toggleLock(clip.id, clip.is_locked);
                }
              }}
              className={`${isMobile ? 'p-3' : 'p-1.5'} rounded-lg transition-colors cursor-pointer ${
                clip.is_locked 
                ? 'text-indigo-500 bg-indigo-50 dark:bg-indigo-500/10' 
                : 'text-slate-400 hover:text-indigo-500 hover:bg-indigo-50 dark:text-gray-400 dark:hover:text-indigo-400 dark:hover:bg-indigo-500/10'
              }`}
            >
              {clip.is_locked ? <ShieldCheck className="w-4 h-4" /> : <Key className="w-4 h-4" />}
            </button>
          </Tooltip>
          <Tooltip text="Copy to clipboard">
            <button
              onClick={() => {
                if (isLocked) {
                  requestUnlock(clip.id, 'copy', false);
                } else {
                  handleCopy(clip);
                }
              }}
              className={`${isMobile ? 'p-3' : 'p-1.5'} hover:bg-indigo-50 dark:hover:bg-indigo-500/10 text-slate-400 hover:text-indigo-500 dark:text-gray-400 dark:hover:text-indigo-400 rounded-lg transition-colors cursor-pointer`}>
              {copiedId === clip.id ? <Check className="w-4 h-4 text-emerald-500" /> : <Copy className="w-4 h-4" />}
            </button>
          </Tooltip>
          <Tooltip text="Delete from history">
            <button
              onClick={() => deleteClip(clip)}
              className={`${isMobile ? 'p-3' : 'p-1.5'} hover:bg-red-50 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500 dark:text-gray-400 dark:hover:text-red-400 rounded-lg transition-colors cursor-pointer`}>
              <Trash2 className="w-4 h-4" />
            </button>
          </Tooltip>
        </div>
      </div>
    </motion.div>
  );
}

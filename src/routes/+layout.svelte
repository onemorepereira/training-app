<script lang="ts">
  import '@fontsource/ibm-plex-mono/latin-400.css';
  import '@fontsource/ibm-plex-mono/latin-500.css';
  import '@fontsource/ibm-plex-mono/latin-600.css';
  import '@fontsource/ibm-plex-mono/latin-700.css';
  import '../app.css';
  import '$lib/ui/controls.css';
  import { page } from '$app/stores';
  import { onMount } from 'svelte';
  import { startSensorListening, stopSensorListening } from '$lib/stores/sensor';
  import { refreshDevices, connectedDevices, handleDeviceDisconnected, handleDeviceReconnecting, handleDeviceReconnected } from '$lib/stores/devices';
  import type { DeviceInfo } from '$lib/tauri';
  import { initAutoSession, destroyAutoSession } from '$lib/stores/autoSession';
  import { rehydrateSession } from '$lib/stores/sessionLifecycle';
  import { unitSystem } from '$lib/stores/units';
  import { api } from '$lib/tauri';
  import { listen } from '@tauri-apps/api/event';
  import { fullscreen, fullscreenError, initFullscreen, destroyFullscreen, setFullscreen, toggleFullscreen } from '$lib/stores/fullscreen';
  import { routeKey, isTypingTarget, overlayOpen } from '$lib/ui/keys';
  import DeviceStatus from '$lib/components/DeviceStatus.svelte';
  import DevToolbar from '$lib/components/DevToolbar.svelte';
  import Notice from '$lib/ui/Notice.svelte';

  let { children } = $props();

  /** The nav shows labels (sidebar) at >= 1100px window width, icons only (rail) below. */
  let wideNav = $state(false);
  let showHint = $state(false);
  let showExit = $state(false);
  let hintTimer: ReturnType<typeof setTimeout> | null = null;
  let exitTimer: ReturnType<typeof setTimeout> | null = null;

  // Entering fullscreen shows a short hint on how to leave it.
  let wasFullscreen = false;
  $effect(() => {
    const fs = $fullscreen;
    if (fs && !wasFullscreen) {
      showHint = true;
      if (hintTimer) clearTimeout(hintTimer);
      hintTimer = setTimeout(() => (showHint = false), 3000);
    }
    if (!fs) showHint = false;
    wasFullscreen = fs;
  });

  function onPointerMove() {
    if (!$fullscreen) return;
    showExit = true;
    if (exitTimer) clearTimeout(exitTimer);
    exitTimer = setTimeout(() => (showExit = false), 2500);
  }

  function onKeydown(e: KeyboardEvent) {
    const action = routeKey(e.key, {
      overlayOpen: overlayOpen(document),
      fullscreen: $fullscreen,
      typing: isTypingTarget(e.target as Element | null),
    });
    if (action === 'none') return;
    e.preventDefault();
    if (action === 'exit-fullscreen') setFullscreen(false);
    else toggleFullscreen();
  }

  onMount(() => {
    startSensorListening();
    // Recover a running/paused session after a webview reload.
    rehydrateSession();
    refreshDevices();
    initAutoSession();
    initFullscreen().catch(() => {});
    api.getUserConfig().then((cfg) => {
      unitSystem.set(cfg.units);
    }).catch(() => {});

    const mq = window.matchMedia('(min-width: 1100px)');
    const onMq = () => (wideNav = mq.matches);
    onMq();
    mq.addEventListener('change', onMq);

    // Global device connection event listeners — store promises so cleanup
    // can unlisten even if the component unmounts before they resolve.
    const listenPromises: Promise<() => void>[] = [];

    listenPromises.push(
      listen<string>('device_disconnected', (event) => {
        handleDeviceDisconnected(event.payload);
      })
    );

    listenPromises.push(
      listen<{ device_id: string; device_type: string; attempt: number }>(
        'device_reconnecting',
        (event) => {
          handleDeviceReconnecting(event.payload);
        }
      )
    );

    listenPromises.push(
      listen<string>('device_reconnected', (event) => {
        handleDeviceReconnected(event.payload);
      })
    );

    listenPromises.push(
      listen<DeviceInfo[]>('device_list_updated', (event) => {
        connectedDevices.set(event.payload);
      })
    );

    window.addEventListener('keydown', onKeydown);
    return () => {
      stopSensorListening();
      destroyAutoSession();
      destroyFullscreen();
      mq.removeEventListener('change', onMq);
      listenPromises.forEach((p) => p.then((fn) => fn()));
      window.removeEventListener('keydown', onKeydown);
      if (hintTimer) clearTimeout(hintTimer);
      if (exitTimer) clearTimeout(exitTimer);
    };
  });

  const navItems = [
    { href: '/', label: 'Ride', icon: 'ride' },
    { href: '/devices', label: 'Devices', icon: 'devices' },
    { href: '/history', label: 'History', icon: 'history' },
    { href: '/analytics', label: 'Analytics', icon: 'analytics' },
    { href: '/settings', label: 'Settings', icon: 'settings' },
    { href: '/about', label: 'About', icon: 'about' },
  ];

  let isRide = $derived($page.url.pathname === '/');
</script>

<svelte:window onpointermove={onPointerMove} />

<div class="app" class:fullscreen={$fullscreen}>
  {#if !$fullscreen}
    <nav class="nav" class:wide={wideNav} aria-label="Main navigation">
      <div class="nav-logo">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="18.5" cy="17.5" r="3.5"/>
          <circle cx="5.5" cy="17.5" r="3.5"/>
          <circle cx="15" cy="5" r="1"/>
          <path d="M12 17.5V14l-3-3 4-3 2 3h2"/>
        </svg>
        {#if wideNav}<span class="nav-brand">Training</span>{/if}
      </div>
      <div class="nav-links">
        {#each navItems as item}
          {@const active = $page.url.pathname === item.href || (item.href !== '/' && $page.url.pathname.startsWith(item.href + '/'))}
          <a
            href={item.href}
            class="nav-item"
            class:active
            aria-current={active ? 'page' : undefined}
            aria-label={wideNav ? undefined : item.label}
          >
            {#if item.icon === 'ride'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="10"/>
                <path d="m10 8 6 4-6 4V8z"/>
              </svg>
            {:else if item.icon === 'devices'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8"/>
                <circle cx="12" cy="12" r="1.5"/>
              </svg>
            {:else if item.icon === 'history'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 12a9 9 0 1 0 3-6.7L3 8"/>
                <path d="M3 3v5h5"/>
                <path d="M12 7v5l3 2"/>
              </svg>
            {:else if item.icon === 'analytics'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <rect x="4" y="13" width="3" height="7" rx="1"/>
                <rect x="10.5" y="8" width="3" height="12" rx="1"/>
                <rect x="17" y="4" width="3" height="16" rx="1"/>
              </svg>
            {:else if item.icon === 'settings'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/>
              </svg>
            {:else if item.icon === 'about'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="10"/>
                <line x1="12" y1="16" x2="12" y2="12"/>
                <line x1="12" y1="8" x2="12.01" y2="8"/>
              </svg>
            {/if}
            {#if wideNav}
              <span class="nav-label">{item.label}</span>
            {:else}
              <span class="nav-tip" role="tooltip" aria-hidden="true">{item.label}</span>
            {/if}
          </a>
        {/each}
      </div>
      <DeviceStatus expanded={wideNav} />
    </nav>
  {/if}

  <main class="content" class:fit={isRide}>
    {#if $fullscreenError}
      <div class="fs-error">
        <Notice tone="error" ondismiss={() => fullscreenError.set('')}>{$fullscreenError}</Notice>
      </div>
    {/if}
    {@render children()}
  </main>

  {#if $fullscreen}
    {#if showHint}
      <div class="fs-hint" role="status">Press <kbd>Esc</kbd> to exit fullscreen</div>
    {/if}
    <button
      type="button"
      class="btn btn-lg btn-secondary fs-exit"
      class:visible={showExit}
      onclick={() => setFullscreen(false)}
    >
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3"/></svg>
      Exit fullscreen
    </button>
  {/if}

  {#if import.meta.env.DEV && isRide}
    <DevToolbar />
  {/if}
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .nav {
    width: var(--nav-rail-width);
    min-width: var(--nav-rail-width);
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border-subtle);
    padding: var(--space-md) var(--space-sm);
    gap: var(--space-xs);
  }

  .nav.wide {
    width: var(--nav-sidebar-width);
    min-width: var(--nav-sidebar-width);
    padding: var(--space-md);
  }

  .nav-logo {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    color: var(--accent);
    padding: var(--space-sm) 0 var(--space-lg);
  }

  .nav.wide .nav-logo {
    justify-content: flex-start;
    padding-left: var(--space-sm);
  }

  .nav-brand {
    color: var(--text-primary);
    font-weight: 700;
    font-size: var(--text-lg);
    letter-spacing: -0.01em;
  }

  .nav-links {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .nav-item {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-md);
    width: 44px;
    height: 44px;
    margin: 0 auto;
    color: var(--text-muted);
    text-decoration: none;
    border-radius: var(--radius-md);
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .nav.wide .nav-item {
    width: 100%;
    justify-content: flex-start;
    padding: 0 var(--space-md);
  }

  .nav-label {
    font-size: var(--text-base);
    font-weight: 600;
  }

  .nav-item:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--text-primary);
  }

  .nav-item.active svg {
    color: var(--accent);
  }

  .nav-item.active::before {
    content: '';
    position: absolute;
    left: -8px;
    top: 25%;
    bottom: 25%;
    width: 3px;
    background: var(--accent);
    border-radius: var(--radius-full);
  }

  .nav.wide .nav-item.active::before {
    left: -12px;
  }

  /* Rail tooltip: shown on hover and on keyboard focus */
  .nav-tip {
    position: absolute;
    left: calc(100% + 10px);
    top: 50%;
    transform: translateY(-50%);
    padding: 4px 10px;
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-default);
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--transition-fast);
    z-index: 200;
    box-shadow: var(--shadow-md);
  }

  .nav-item:hover .nav-tip,
  .nav-item:focus-visible .nav-tip {
    opacity: 1;
  }

  .content {
    position: relative;
    flex: 1;
    padding: var(--space-xl) var(--space-2xl);
    overflow-y: auto;
    min-width: 0;
  }

  /* Ride: exactly the window height, never scrolls (its controls must stay visible) */
  .content.fit {
    overflow: hidden;
    padding: var(--space-lg);
    display: flex;
    flex-direction: column;
  }

  .fs-error {
    margin-bottom: var(--space-md);
  }

  .fs-hint {
    position: fixed;
    top: 24px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 300;
    padding: 10px 18px;
    border-radius: var(--radius-full);
    border: 1px solid var(--border-strong);
    background: rgba(28, 28, 48, 0.95);
    color: var(--text-primary);
    font-size: var(--text-base);
    box-shadow: var(--shadow-lg);
    animation: fade-in 200ms ease;
  }

  .fs-hint kbd {
    padding: 1px 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    font-family: var(--font-data);
    font-size: var(--text-sm);
  }

  /* Hidden until the pointer moves; stays reachable by keyboard (opacity only) */
  .fs-exit {
    position: fixed;
    top: 16px;
    right: 16px;
    z-index: 300;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--transition-base);
  }

  .fs-exit.visible,
  .fs-exit:focus-visible {
    opacity: 1;
    pointer-events: auto;
  }
</style>

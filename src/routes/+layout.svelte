<script lang="ts">
  import '../app.css';
  import { page } from '$app/stores';
  import { onMount, onDestroy } from 'svelte';
  import { startSensorListening, stopSensorListening } from '$lib/stores/sensor';
  import { refreshDevices, handleDeviceDisconnected, handleDeviceReconnecting, handleDeviceReconnected } from '$lib/stores/devices';
  import { initAutoSession, destroyAutoSession } from '$lib/stores/autoSession';
  import { unitSystem } from '$lib/stores/units';
  import { api } from '$lib/tauri';
  import { listen } from '@tauri-apps/api/event';
  import DeviceStatus from '$lib/components/DeviceStatus.svelte';

  let { children } = $props();
  let navCollapsed = $state(false);
  let onKeydown: ((e: KeyboardEvent) => void) | null = null;
  let onToggleFullscreen: (() => void) | null = null;

  onMount(() => {
    startSensorListening();
    refreshDevices();
    initAutoSession();
    api.getUserConfig().then((cfg) => {
      unitSystem.set(cfg.units);
    }).catch(() => {});

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
        refreshDevices();
      })
    );

    onKeydown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        navCollapsed = !navCollapsed;
      }
    };
    onToggleFullscreen = () => {
      navCollapsed = !navCollapsed;
    };
    window.addEventListener('keydown', onKeydown);
    window.addEventListener('toggle-fullscreen', onToggleFullscreen);
    return () => {
      stopSensorListening();
      destroyAutoSession();
      listenPromises.forEach((p) => p.then((fn) => fn()));
      if (onKeydown) window.removeEventListener('keydown', onKeydown);
      if (onToggleFullscreen) window.removeEventListener('toggle-fullscreen', onToggleFullscreen);
    };
  });

  const navItems = [
    { href: '/', label: 'Ride', icon: 'ride' },
    { href: '/devices', label: 'Devices', icon: 'devices' },
    { href: '/history', label: 'History', icon: 'history' },
    { href: '/analytics', label: 'Analytics', icon: 'analytics' },
    { href: '/settings', label: 'Settings', icon: 'settings' },
  ];
</script>

<div class="app">
  {#if !navCollapsed}
    <nav class="nav-rail">
      <div class="rail-logo">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="5" cy="18" r="3"/>
          <circle cx="19" cy="18" r="3"/>
          <path d="M12 2l-3.5 7H15l-2 6"/>
          <line x1="8" y1="18" x2="16" y2="18"/>
          <line x1="12" y1="9" x2="19" y2="18"/>
          <line x1="5" y1="18" x2="8.5" y2="9"/>
        </svg>
      </div>
      <div class="rail-links">
        {#each navItems as item}
          <a
            href={item.href}
            class="rail-item"
            class:active={$page.url.pathname === item.href}
            data-tooltip={item.label}
          >
            {#if item.icon === 'ride'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="4 14 8 10 12 14 16 8 20 12"/>
              </svg>
            {:else if item.icon === 'devices'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 20v-6"/>
                <path d="M12 10c-3.3 0-6-2.7-6-6"/>
                <path d="M12 10c3.3 0 6-2.7 6-6"/>
                <path d="M12 10c-1.7 0-3-1.3-3-3"/>
                <path d="M12 10c1.7 0 3-1.3 3-3"/>
              </svg>
            {:else if item.icon === 'history'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <rect x="4" y="14" width="3" height="6" rx="1"/>
                <rect x="10.5" y="8" width="3" height="12" rx="1"/>
                <rect x="17" y="4" width="3" height="16" rx="1"/>
              </svg>
            {:else if item.icon === 'analytics'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
              </svg>
            {:else if item.icon === 'settings'}
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/>
              </svg>
            {/if}
          </a>
        {/each}
      </div>
      <DeviceStatus compact={true} />
    </nav>
  {/if}
  <main class="content">
    {@render children()}
  </main>
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .nav-rail {
    width: var(--nav-rail-width);
    min-width: var(--nav-rail-width);
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
    align-items: center;
    border-right: 1px solid var(--border-subtle);
    position: relative;
    padding: var(--space-md) 0;
  }

  .nav-rail::after {
    content: '';
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 1px;
    background: linear-gradient(
      180deg,
      transparent 0%,
      var(--accent-soft) 30%,
      var(--accent-soft) 70%,
      transparent 100%
    );
    pointer-events: none;
  }

  .rail-logo {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    padding: var(--space-sm) 0 var(--space-lg);
  }

  .rail-links {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    padding: 0 var(--space-sm);
  }

  .rail-item {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    margin: 0 auto;
    color: var(--text-muted);
    text-decoration: none;
    border-radius: var(--radius-md);
    transition: all var(--transition-fast);
    position: relative;
  }

  .rail-item:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .rail-item.active {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .rail-item.active::before {
    content: '';
    position: absolute;
    left: -8px;
    top: 25%;
    bottom: 25%;
    width: 3px;
    background: var(--accent);
    border-radius: var(--radius-full);
  }

  /* CSS tooltip */
  .rail-item::after {
    content: attr(data-tooltip);
    position: absolute;
    left: calc(100% + 8px);
    top: 50%;
    transform: translateY(-50%);
    padding: 4px 8px;
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--transition-fast);
    border: 1px solid var(--border-default);
    z-index: 100;
  }

  .rail-item:hover::after {
    opacity: 1;
  }

  .content {
    flex: 1;
    padding: var(--space-xl) var(--space-2xl);
    overflow-y: auto;
    min-width: 0;
  }
</style>

/**
 * @vitest-environment jsdom
 */
import { beforeEach, describe, expect, it } from 'vitest'
import {
  getRememberedChatRoute,
  getRememberedChatSidebarCollapsed,
  hashPath,
  isChatPath,
} from './persistence'

beforeEach(() => window.localStorage.clear())

describe('hashPath', () => {
  it('strips hash prefix and query string', () => {
    window.location.hash = '#chat/settings?tab=general'
    expect(hashPath()).toBe('chat/settings')
  })
})

describe('isChatPath', () => {
  it('matches chat routes', () => {
    expect(isChatPath('chat')).toBe(true)
    expect(isChatPath('chat/conv-1')).toBe(true)
    expect(isChatPath('settings')).toBe(false)
  })
})

describe('legacy Beefex renderer storage migration', () => {
  it('moves the remembered route to the Beefex key on read', () => {
    window.localStorage.setItem('kivio-chat-last-route', '#chat/task-1')

    expect(getRememberedChatRoute()).toBe('#chat/task-1')
    expect(window.localStorage.getItem('beefex-chat-last-route')).toBe('#chat/task-1')
    expect(window.localStorage.getItem('kivio-chat-last-route')).toBeNull()
  })

  it('moves the sidebar preference to the Beefex key on read', () => {
    window.localStorage.setItem('kivio-chat-sidebar-collapsed', '1')

    expect(getRememberedChatSidebarCollapsed()).toBe(true)
    expect(window.localStorage.getItem('beefex-chat-sidebar-collapsed')).toBe('1')
    expect(window.localStorage.getItem('kivio-chat-sidebar-collapsed')).toBeNull()
  })
})

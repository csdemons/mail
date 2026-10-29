import { useState } from 'react'
import { Copy, Search, SquarePen, UserPlus } from 'lucide-react'

import { api, errorText } from './api'
import ContactEditor from './ContactEditor'
import ContextMenu from './ContextMenu'
import type { DavCollection } from './types'

/** «Имя <адрес>» или просто адрес → имя и адрес. */
export function parseAddress(entry: string): { name: string; address: string } {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]+)>\s*$/.exec(entry)
  if (m) return { name: m[1].trim(), address: m[2].trim() }
  return { name: '', address: entry.trim() }
}

/** Копирование в буфер: Clipboard API, а где он недоступен (WebKitGTK) — через выделение. */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    return
  } catch { /* запасной путь ниже */ }
  const area = document.createElement('textarea')
  area.value = text
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.appendChild(area)
  area.select()
  document.execCommand('copy')
  area.remove()
}

/** Адреса письма: выделяются и копируются как текст; щелчок — меню, как в Outlook:
 *  скопировать, написать, добавить в контакты, найти письма. */
export default function AddressLine({ entries, strong, onCompose, onSearch, onNotice }: {
  entries: string[]
  strong?: boolean
  onCompose?: (to: string) => void
  onSearch?: (address: string) => void
  onNotice: (text: string) => void
}) {
  const [menu, setMenu] = useState<{ x: number; y: number; entry: string } | null>(null)
  const [adding, setAdding] = useState<{ name: string; address: string; books: DavCollection[] } | null>(null)

  async function addContact(entry: string) {
    const { name, address } = parseAddress(entry)
    try {
      const existing = await api.searchContacts(address).catch(() => [])
      if (existing.some((c) => c.emails.some((e) => e.toLowerCase() === address.toLowerCase()))) {
        onNotice(`${address} уже есть в контактах.`)
        return
      }
      const { books } = await api.listContacts()
      if (!books.some((b) => b.writable)) {
        onNotice('Нет адресной книги, в которую можно добавить контакт.')
        return
      }
      setAdding({ name: name || address.split('@')[0], address, books })
    } catch (error) {
      onNotice(`Не удалось открыть контакты: ${errorText(error)}`)
    }
  }

  return <>
    {entries.map((entry, i) => {
      const { name, address } = parseAddress(entry)
      return <span key={`${entry}-${i}`} className="address-item">
        <span
          className={`address-chip ${strong ? 'strong' : ''}`}
          role="button"
          tabIndex={0}
          title={`${address} — щелчок: скопировать, написать, в контакты`}
          data-address={address}
          onClick={(e) => {
            // Выделение текста мышью не открывает меню.
            if (window.getSelection()?.toString()) return
            setMenu({ x: e.clientX, y: e.clientY, entry })
          }}
          onContextMenu={(e) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, entry }) }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              const r = (e.target as HTMLElement).getBoundingClientRect()
              setMenu({ x: r.left, y: r.bottom, entry })
            }
          }}
        >
          {name ? <>{name} <span className="address-raw">&lt;{address}&gt;</span></> : address}
        </span>
        {i < entries.length - 1 && ', '}
      </span>
    })}
    {menu && <ContextMenu
      x={menu.x}
      y={menu.y}
      onClose={() => setMenu(null)}
      items={[
        { label: 'Копировать адрес', icon: <Copy size={15} />, onSelect: () => { const a = parseAddress(menu.entry).address; void copyText(a).then(() => onNotice(`Скопировано: ${a}`)) } },
        ...(parseAddress(menu.entry).name ? [{ label: 'Копировать имя и адрес', icon: <Copy size={15} />, onSelect: () => { void copyText(menu.entry).then(() => onNotice('Скопировано.')) } }] : []),
        { separator: true as const },
        ...(onCompose ? [{ label: 'Написать письмо', icon: <SquarePen size={15} />, onSelect: () => onCompose(menu.entry) }] : []),
        { label: 'Добавить в контакты…', icon: <UserPlus size={15} />, onSelect: () => void addContact(menu.entry) },
        ...(onSearch ? [{ label: 'Найти письма с этим адресом', icon: <Search size={15} />, onSelect: () => onSearch(parseAddress(menu.entry).address) }] : []),
      ]}
    />}
    {adding && <ContactEditor
      contact={null}
      prefill={{ fullName: adding.name, emails: [adding.address] }}
      books={adding.books}
      defaultBook=""
      onClose={() => setAdding(null)}
      onSaved={(text) => { setAdding(null); onNotice(text) }}
    />}
  </>
}

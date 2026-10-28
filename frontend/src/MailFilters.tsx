import { useEffect, useMemo, useState } from 'react'
import { ArrowDown, ArrowUp, FolderInput, Pencil, Plus, Trash2, X } from 'lucide-react'
import { api, errorText } from './api'
import FolderPicker from './FolderPicker'
import type { Folder, MessageSummary, SieveAction, SieveFilter, SieveRule } from './types'

const FIELDS: [string, string][] = [
  ['from', 'От'], ['to', 'Кому'], ['cc', 'Копия'], ['to_or_cc', 'Кому или копия'],
  ['subject', 'Тема'], ['body', 'Текст письма'], ['size', 'Размер (КБ)'], ['header', 'Заголовок…'],
]
const TEXT_OPS: [string, string][] = [
  ['contains', 'содержит'], ['not_contains', 'не содержит'], ['is', 'равно'], ['is_not', 'не равно'],
  ['matches', 'совпадает с шаблоном (*, ?)'], ['not_matches', 'не совпадает с шаблоном'],
  ['regex', 'регулярное выражение'], ['not_regex', 'не регулярное выражение'],
]
const SIZE_OPS: [string, string][] = [['over', 'больше'], ['under', 'меньше']]
const ACTIONS: [string, string][] = [
  ['fileinto', 'Переместить в папку'], ['addflag', 'Пометить'], ['redirect', 'Перенаправить на адрес'],
  ['keep', 'Оставить во «Входящих»'], ['discard', 'Удалить'], ['stop', 'Не применять другие фильтры'],
]
const FLAGS: [string, string][] = [
  ['seen', 'прочитанным'], ['flagged', 'важным (звёздочка)'], ['answered', 'отвеченным'],
  ['junk', 'спамом'], ['not_junk', 'не спамом'], ['deleted', 'удалённым'],
]
const NO_ARG = new Set(['keep', 'discard', 'stop'])

const isActive = (f: SieveFilter) => f.active === true || f.active === 1 || f.active === '1'

function describe(f: SieveFilter, folderName: (path: string) => string): string {
  const cond = f.match === 'allmessages' ? 'все письма' : f.rules.map((r) => {
    const field = r.field === 'header' ? (r.custom_header || 'заголовок') : (FIELDS.find(([k]) => k === r.field)?.[1] ?? r.field)
    const op = [...TEXT_OPS, ...SIZE_OPS].find(([k]) => k === r.operator)?.[1] ?? r.operator
    return `«${field}» ${op} «${r.value}»`
  }).join(f.match === 'any' ? ' или ' : ' и ')
  const act = f.actions.map((a) => {
    if (a.method === 'fileinto') return `→ ${folderName(a.argument ?? '')}`
    if (a.method === 'addflag') return `пометить ${FLAGS.find(([k]) => k === a.argument)?.[1] ?? a.argument}`
    if (a.method === 'redirect') return `→ ${a.argument}`
    return ACTIONS.find(([k]) => k === a.method)?.[1].toLowerCase() ?? a.method
  }).join(', ')
  return `${cond}: ${act}`
}

function blank(seed: MessageSummary | null): SieveFilter {
  if (seed) {
    return {
      name: `От ${seed.fromName || seed.fromAddress}`,
      active: 1,
      match: 'all',
      rules: [{ field: 'from', operator: 'contains', value: seed.fromAddress }],
      actions: [{ method: 'fileinto', argument: '' }],
    }
  }
  return { name: '', active: 1, match: 'all', rules: [{ field: 'subject', operator: 'contains', value: '' }], actions: [{ method: 'fileinto', argument: '' }] }
}

interface Props {
  folders: Folder[]
  seed: MessageSummary | null
  label: (f: Folder) => string
  onClose: () => void
  onNotice: (text: string) => void
}

/** Фильтры почты — правила Sieve на сервере (SOGo), работают и без запущенного клиента. */
export default function MailFilters({ folders, seed, label, onClose, onNotice }: Props) {
  const [filters, setFilters] = useState<SieveFilter[] | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState<{ index: number | null; filter: SieveFilter } | null>(seed ? { index: null, filter: blank(seed) } : null)

  useEffect(() => {
    api.mailFiltersGet()
      .then((r) => setFilters(Array.isArray(r.filters) ? r.filters : []))
      .catch((e) => setError(errorText(e)))
  }, [])

  const byPath = useMemo(() => new Map(folders.map((f) => [f.path, f])), [folders])
  const folderName = (path: string) => {
    const f = byPath.get(path)
    return f ? (f.depth > 0 ? f.path : label(f)) : path || '(папка не выбрана)'
  }

  async function save(next: SieveFilter[], text: string) {
    setBusy(true)
    try {
      await api.mailFiltersSave(next)
      setFilters(next)
      onNotice(text)
      return true
    } catch (e) {
      onNotice(`Фильтры не сохранены: ${errorText(e)}`)
      return false
    } finally {
      setBusy(false)
    }
  }

  async function commit(f: SieveFilter, index: number | null) {
    if (!filters) return
    const next = [...filters]
    if (index === null) next.push(f)
    else next[index] = f
    if (await save(next, index === null ? `Фильтр «${f.name}» создан.` : `Фильтр «${f.name}» сохранён.`)) setEditing(null)
  }

  function toggle(i: number) {
    if (!filters) return
    const f = filters[i]
    const on = !isActive(f)
    const active = typeof f.active === 'boolean' ? on : on ? 1 : 0
    void save(filters.map((x, j) => (j === i ? { ...x, active } : x)), on ? `Фильтр «${f.name}» включён.` : `Фильтр «${f.name}» выключен.`)
  }

  function moveFilter(i: number, d: -1 | 1) {
    if (!filters) return
    const j = i + d
    if (j < 0 || j >= filters.length) return
    const next = [...filters]
    ;[next[i], next[j]] = [next[j], next[i]]
    void save(next, 'Порядок фильтров изменён.')
  }

  function remove(i: number) {
    if (!filters) return
    const f = filters[i]
    void save(filters.filter((_, j) => j !== i), `Фильтр «${f.name}» удалён.`)
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !editing) onClose() }}>
      <div className="settings-modal filters-modal" role="dialog" aria-label="Фильтры почты">
        <div className="compose-header">
          <div><strong>Фильтры почты</strong><small>Правила выполняются на сервере для каждого нового письма — даже когда программа закрыта</small></div>
          <button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        {error && <div className="inline-warning" role="alert">Не удалось загрузить фильтры: {error}</div>}
        {!error && filters === null && <div className="folder-placeholder">Загрузка…</div>}
        {editing ? (
          <FilterEditor
            key={editing.index ?? 'new'}
            initial={editing.filter}
            folders={folders}
            label={label}
            folderName={folderName}
            busy={busy || filters === null}
            onCancel={() => setEditing(null)}
            onSave={(f) => void commit(f, editing.index)}
          />
        ) : filters && <>
          <div className="filter-list">
            {filters.length === 0 && <div className="folder-placeholder">Фильтров пока нет. Например: письма от рассылки — в отдельную папку.</div>}
            {filters.map((f, i) => (
              <div key={i} className={`filter-item ${isActive(f) ? '' : 'off'}`}>
                <label className="switch" title={isActive(f) ? 'Выключить' : 'Включить'}>
                  <input type="checkbox" aria-label={`Включён: ${f.name}`} checked={isActive(f)} disabled={busy} onChange={() => toggle(i)} />
                  <span />
                </label>
                <div className="filter-text">
                  <strong>{f.name}</strong>
                  <span>{describe(f, folderName)}</span>
                </div>
                <button className="icon-button soft" aria-label="Выше" disabled={busy || i === 0} onClick={() => moveFilter(i, -1)}><ArrowUp size={15} /></button>
                <button className="icon-button soft" aria-label="Ниже" disabled={busy || i === filters.length - 1} onClick={() => moveFilter(i, 1)}><ArrowDown size={15} /></button>
                <button className="icon-button soft" aria-label={`Изменить ${f.name}`} disabled={busy} onClick={() => setEditing({ index: i, filter: f })}><Pencil size={15} /></button>
                <button className="icon-button soft danger-icon" aria-label={`Удалить ${f.name}`} disabled={busy} onClick={() => remove(i)}><Trash2 size={15} /></button>
              </div>
            ))}
          </div>
          <div className="settings-actions">
            <button className="primary-button" disabled={busy} onClick={() => setEditing({ index: null, filter: blank(null) })}><Plus size={15} /> Новый фильтр</button>
          </div>
        </>}
      </div>
    </div>
  )
}

function FilterEditor({ initial, folders, label, folderName, busy, onCancel, onSave }: {
  initial: SieveFilter
  folders: Folder[]
  label: (f: Folder) => string
  folderName: (path: string) => string
  busy: boolean
  onCancel: () => void
  onSave: (f: SieveFilter) => void
}) {
  const [f, setF] = useState<SieveFilter>(() => JSON.parse(JSON.stringify(initial)) as SieveFilter)
  const [pickFor, setPickFor] = useState<number | null>(null)
  const [problem, setProblem] = useState('')

  const setRule = (i: number, patch: Partial<SieveRule>) => setF((x) => ({ ...x, rules: x.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }))
  const setAction = (i: number, patch: Partial<SieveAction>) => setF((x) => ({ ...x, actions: x.actions.map((a, j) => (j === i ? { ...a, ...patch } : a)) }))

  function submit() {
    const name = f.name.trim()
    if (!name) return setProblem('Укажите название фильтра.')
    const rules = f.match === 'allmessages' ? [] : f.rules.map((r) => ({ ...r, value: r.value.trim() }))
    if (f.match !== 'allmessages' && (rules.length === 0 || rules.some((r) => !r.value))) return setProblem('Заполните значения условий.')
    if (rules.some((r) => r.field === 'header' && !r.custom_header?.trim())) return setProblem('Укажите имя заголовка.')
    if (rules.some((r) => r.field === 'size' && !/^\d+$/.test(r.value))) return setProblem('Размер — целое число килобайт.')
    const actions: SieveAction[] = f.actions.map((a) => (NO_ARG.has(a.method) ? { method: a.method } : { ...a, argument: (a.argument ?? '').trim() }))
    if (actions.length === 0) return setProblem('Добавьте хотя бы одно действие.')
    if (actions.some((a) => a.method === 'fileinto' && !a.argument)) return setProblem('Выберите папку для перемещения.')
    if (actions.some((a) => a.method === 'redirect' && !/^[^\s@]+@[^\s@]+$/.test(a.argument ?? ''))) return setProblem('Укажите адрес для перенаправления.')
    onSave({ ...f, name, rules, actions })
  }

  return (
    <div className="filter-editor">
      <label className="login-field"><span>Название</span><input aria-label="Название фильтра" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="Например: Рассылка НОЦ" /></label>
      <label className="login-field"><span>Применять, если</span>
        <select aria-label="Условие" value={f.match} onChange={(e) => setF({ ...f, match: e.target.value as SieveFilter['match'], rules: f.rules.length ? f.rules : [{ field: 'subject', operator: 'contains', value: '' }] })}>
          <option value="all">выполнены все условия</option>
          <option value="any">выполнено любое условие</option>
          <option value="allmessages">к каждому письму</option>
        </select>
      </label>
      {f.match !== 'allmessages' && <div className="filter-rows">
        {f.rules.map((r, i) => (
          <div key={i} className="filter-row">
            <select aria-label="Поле" value={r.field} onChange={(e) => {
              const field = e.target.value
              setRule(i, { field, operator: field === 'size' ? 'over' : (r.field === 'size' ? 'contains' : r.operator) })
            }}>
              {FIELDS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
            {r.field === 'header' && <input aria-label="Имя заголовка" className="filter-header" value={r.custom_header ?? ''} onChange={(e) => setRule(i, { custom_header: e.target.value })} placeholder="List-Id" />}
            <select aria-label="Сравнение" value={r.operator} onChange={(e) => setRule(i, { operator: e.target.value })}>
              {(r.field === 'size' ? SIZE_OPS : TEXT_OPS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
            <input aria-label="Значение" value={r.value} onChange={(e) => setRule(i, { value: e.target.value })} placeholder={r.field === 'size' ? '1024' : 'текст'} />
            <button className="icon-button soft" aria-label="Убрать условие" disabled={f.rules.length < 2} onClick={() => setF({ ...f, rules: f.rules.filter((_, j) => j !== i) })}><X size={15} /></button>
          </div>
        ))}
        <button className="text-button" onClick={() => setF({ ...f, rules: [...f.rules, { field: 'subject', operator: 'contains', value: '' }] })}><Plus size={14} /> Условие</button>
      </div>}
      <span className="filter-caption">Действия</span>
      <div className="filter-rows">
        {f.actions.map((a, i) => (
          <div key={i} className="filter-row">
            <select aria-label="Действие" value={a.method} onChange={(e) => {
              const method = e.target.value
              setAction(i, { method, argument: method === 'addflag' ? 'seen' : method === a.method ? a.argument : '' })
            }}>
              {ACTIONS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
            {a.method === 'fileinto' && <button className="secondary-button filter-folder" onClick={() => setPickFor(i)}><FolderInput size={15} /> {a.argument ? folderName(a.argument) : 'Выбрать папку…'}</button>}
            {a.method === 'addflag' && <select aria-label="Пометка" value={a.argument ?? 'seen'} onChange={(e) => setAction(i, { argument: e.target.value })}>
              {FLAGS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>}
            {a.method === 'redirect' && <input aria-label="Адрес" value={a.argument ?? ''} onChange={(e) => setAction(i, { argument: e.target.value })} placeholder="ivanov@nrcki.ru" />}
            <button className="icon-button soft" aria-label="Убрать действие" disabled={f.actions.length < 2} onClick={() => setF({ ...f, actions: f.actions.filter((_, j) => j !== i) })}><X size={15} /></button>
          </div>
        ))}
        <button className="text-button" onClick={() => setF({ ...f, actions: [...f.actions, { method: 'addflag', argument: 'seen' }] })}><Plus size={14} /> Действие</button>
      </div>
      {problem && <div className="inline-warning" role="alert">{problem}</div>}
      <div className="settings-actions">
        <button className="secondary-button" onClick={onCancel}>Отмена</button>
        <button className="primary-button" disabled={busy} onClick={submit}>Сохранить фильтр</button>
      </div>
      {pickFor !== null && <FolderPicker
        folders={folders}
        personalOnly
        title="Папка для писем"
        label={label}
        onClose={() => setPickFor(null)}
        onPick={(id) => {
          const path = folders.find((x) => x.id === id)?.path ?? id
          setAction(pickFor, { argument: path })
          setPickFor(null)
          setProblem('')
        }}
      />}
    </div>
  )
}

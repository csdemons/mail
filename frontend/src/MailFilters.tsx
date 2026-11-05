import { useEffect, useMemo, useState } from 'react'
import { ArrowDown, ArrowUp, Cloud, FolderInput, Monitor, Pencil, Plus, Trash2, X } from 'lucide-react'
import { api, errorText } from './api'
import FolderPicker from './FolderPicker'
import type { Category, DeckBoard, DeckStack, Folder, MessageSummary, SieveAction, SieveFilter, SieveRule } from './types'

const FIELDS: [string, string][] = [
  ['from', 'От'], ['to', 'Кому'], ['cc', 'Копия'], ['to_or_cc', 'Кому или копия'],
  ['subject', 'Тема'], ['body', 'Текст письма (ключевые слова)'], ['domain', 'Подразделение (домен)'],
  ['attachment', 'Имя вложения'], ['has_attachment', 'Есть вложение'],
  ['size', 'Размер (КБ)'], ['header', 'Заголовок…'],
]
/** Условия, которые проверяет только программа (сервер их не умеет). */
const PROGRAM_FIELDS = new Set(['attachment', 'has_attachment'])
const DOMAIN_OPS: [string, string][] = [['is', 'равно'], ['is_not', 'не равно']]
const TEXT_OPS: [string, string][] = [
  ['contains', 'содержит'], ['not_contains', 'не содержит'], ['is', 'равно'], ['is_not', 'не равно'],
  ['matches', 'совпадает с шаблоном (*, ?)'], ['not_matches', 'не совпадает с шаблоном'],
  ['regex', 'регулярное выражение'], ['not_regex', 'не регулярное выражение'],
]
const SIZE_OPS: [string, string][] = [['over', 'больше'], ['under', 'меньше']]
const ACTIONS: [string, string][] = [
  ['fileinto', 'Переместить в папку'], ['addflag', 'Пометить'], ['category', 'Категория'],
  ['notify', 'Уведомить'], ['task', 'Создать задачу в Deck'], ['redirect', 'Перенаправить на адрес'],
  ['keep', 'Оставить во «Входящих»'], ['discard', 'Удалить'], ['stop', 'Не применять другие правила'],
]
/** Действия, которые выполняет программа (сервер их не умеет). */
const PROGRAM_ACTIONS = new Set(['notify', 'task'])
/** Только на сервере: программа не пересылает и не удаляет письма. */
const SERVER_ONLY_ACTIONS = new Set(['redirect', 'keep', 'discard'])
const LABEL_KW = /^\$label([1-5])$/

type Where = 'server' | 'program'

/** Действие программы: уведомление, задача и своя (не стандартная) категория. */
function isProgramAction(a: SieveAction): boolean {
  return PROGRAM_ACTIONS.has(a.method) || (a.method === 'category' && !LABEL_KW.test(a.argument ?? ''))
}

/** Где будет выполняться правило: условия по вложениям проверяет только программа. */
export function placementOf(f: SieveFilter, serverAvailable: boolean): Where {
  if (!serverAvailable) return 'program'
  return f.match !== 'allmessages' && f.rules.some((r) => PROGRAM_FIELDS.has(r.field)) ? 'program' : 'server'
}

/** Фильтр с сервера → вид для редактора (домен и стандартные метки — по-человечески). */
function fromServer(f: SieveFilter, ext: SieveAction[]): SieveFilter {
  const rules = f.rules.map((r) => (r.field === 'from' && r.operator === 'contains' && /^@[^\s@]+$/.test(r.value) ? { field: 'domain', operator: 'is', value: r.value.slice(1) } : r))
  const actions = f.actions.map((a) => (a.method === 'addflag' && /^label[1-5]$/.test(a.argument ?? '') ? { method: 'category', argument: `$${a.argument}` } : a))
  return { ...f, rules, actions: [...actions, ...ext] }
}

/** Вид редактора → фильтр для сервера (без действий программы). */
function toServer(f: SieveFilter): SieveFilter {
  const rules = f.rules.map((r) => (r.field === 'domain' ? { field: 'from', operator: r.operator === 'is_not' ? 'not_contains' : 'contains', value: `@${r.value.replace(/^@/, '')}` } : r))
  const actions = f.actions.filter((a) => !isProgramAction(a)).map((a) => (a.method === 'category' ? { method: 'addflag', argument: (a.argument ?? '').slice(1) } : a))
  return { ...f, rules, actions }
}
const FLAGS: [string, string][] = [
  ['seen', 'прочитанным'], ['flagged', 'важным (звёздочка)'], ['answered', 'отвеченным'],
  ['junk', 'спамом'], ['not_junk', 'не спамом'], ['deleted', 'удалённым'],
]
const NO_ARG = new Set(['keep', 'discard', 'stop', 'notify'])

const isActive = (f: SieveFilter) => f.active === true || f.active === 1 || f.active === '1'

function describe(f: SieveFilter, folderName: (path: string) => string, catName: (kw: string) => string): string {
  const cond = f.match === 'allmessages' ? 'все письма' : f.rules.map((r) => {
    if (r.field === 'has_attachment') return 'есть вложение'
    const field = r.field === 'header' ? (r.custom_header || 'заголовок') : (FIELDS.find(([k]) => k === r.field)?.[1] ?? r.field)
    const op = [...TEXT_OPS, ...SIZE_OPS].find(([k]) => k === r.operator)?.[1] ?? r.operator
    return `«${field}» ${op} «${r.value}»`
  }).join(f.match === 'any' ? ' или ' : ' и ')
  const act = f.actions.map((a) => {
    if (a.method === 'fileinto') return `→ ${folderName(a.argument ?? '')}`
    if (a.method === 'addflag') return `пометить ${FLAGS.find(([k]) => k === a.argument)?.[1] ?? a.argument}`
    if (a.method === 'category') return `категория «${catName(a.argument ?? '')}»`
    if (a.method === 'redirect') return `→ ${a.argument}`
    if (a.method === 'task') return 'задача в Deck'
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
  categories: Category[]
  label: (f: Folder) => string
  onClose: () => void
  onNotice: (text: string) => void
}

interface Item { where: Where; index: number; filter: SieveFilter }

/** Правила обработки почты. То, что умеет сервер (Sieve в SOGo), работает на сервере
 *  всегда, даже когда программа закрыта; условия по вложениям, уведомления и задачи
 *  выполняет программа, пока она запущена. */
export default function MailFilters({ folders, seed, categories, label, onClose, onNotice }: Props) {
  /** Серверные правила (с действиями программы, если есть). null — ещё загружаются. */
  const [server, setServer] = useState<SieveFilter[] | null>(null)
  const [serverError, setServerError] = useState('')
  const [program, setProgram] = useState<SieveFilter[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState<{ item: Item | null; filter: SieveFilter } | null>(seed ? { item: null, filter: blank(seed) } : null)

  useEffect(() => {
    let ext: Record<string, SieveAction[]> = {}
    const local = api.rulesGet().then((r) => { ext = r.ext ?? {}; setProgram(r.program ?? []) }).catch((e) => { setProgram([]); onNotice(`Правила программы не загружены: ${errorText(e)}`) })
    api.mailFiltersGet()
      .then(async (r) => { await local; setServer((Array.isArray(r.filters) ? r.filters : []).map((f) => fromServer(f, ext[f.name] ?? []))) })
      .catch((e) => { setServerError(errorText(e)); setServer([]) })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const serverOk = !serverError
  const byPath = useMemo(() => new Map(folders.map((f) => [f.path, f])), [folders])
  const folderName = (path: string) => {
    const f = byPath.get(path)
    return f ? (f.depth > 0 ? f.path : label(f)) : path || '(папка не выбрана)'
  }
  const catName = (kw: string) => categories.find((c) => c.keyword.toLowerCase() === kw.toLowerCase())?.name ?? kw
  const items: Item[] = [
    ...(server ?? []).map((filter, index) => ({ where: 'server' as const, index, filter })),
    ...(program ?? []).map((filter, index) => ({ where: 'program' as const, index, filter })),
  ]

  /** Сохранить оба списка: серверные — в SOGo (без действий программы), остальное — в программе. */
  async function persist(nextServer: SieveFilter[], nextProgram: SieveFilter[], text: string, serverChanged = true) {
    setBusy(true)
    try {
      if (serverChanged && serverOk) await api.mailFiltersSave(nextServer.map(toServer))
      const ext: Record<string, SieveAction[]> = {}
      for (const f of nextServer) {
        const acts = f.actions.filter(isProgramAction)
        if (acts.length) ext[f.name] = acts
      }
      await api.rulesSave({ program: nextProgram, ext })
      setServer(nextServer)
      setProgram(nextProgram)
      onNotice(text)
      return true
    } catch (e) {
      onNotice(`Правила не сохранены: ${errorText(e)}`)
      return false
    } finally {
      setBusy(false)
    }
  }

  async function commit(f: SieveFilter, item: Item | null) {
    if (!server || !program) return
    const where = placementOf(f, serverOk)
    const s = [...server]
    const p = [...program]
    if (item) (item.where === 'server' ? s : p).splice(item.index, 1)
    // Новое (или перенесённое) правило — в конец своего списка, прежнее — на своё место.
    const list = where === 'server' ? s : p
    if (item && item.where === where) list.splice(item.index, 0, f)
    else list.push(f)
    const serverChanged = where === 'server' || item?.where === 'server'
    const text = where === 'server'
      ? `Правило «${f.name}» сохранено на сервере${f.actions.some(isProgramAction) ? ' (уведомление и задачу добавляет программа)' : ''}.`
      : `Правило «${f.name}» сохранено — его выполняет программа, пока она запущена.`
    if (await persist(s, p, text, serverChanged)) setEditing(null)
  }

  function update(item: Item, change: (list: SieveFilter[]) => SieveFilter[], text: string) {
    if (!server || !program) return
    if (item.where === 'server') void persist(change(server), program, text, true)
    else void persist(server, change(program), text, false)
  }

  function toggle(item: Item) {
    const f = item.filter
    const on = !isActive(f)
    const active = typeof f.active === 'boolean' ? on : on ? 1 : 0
    update(item, (list) => list.map((x, j) => (j === item.index ? { ...x, active } : x)), on ? `Правило «${f.name}» включено.` : `Правило «${f.name}» выключено.`)
  }

  function moveItem(item: Item, d: -1 | 1) {
    update(item, (list) => {
      const j = item.index + d
      if (j < 0 || j >= list.length) return list
      const next = [...list]
      ;[next[item.index], next[j]] = [next[j], next[item.index]]
      return next
    }, 'Порядок правил изменён.')
  }

  function remove(item: Item) {
    update(item, (list) => list.filter((_, j) => j !== item.index), `Правило «${item.filter.name}» удалено.`)
  }

  const listLen = (w: Where) => (w === 'server' ? server?.length ?? 0 : program?.length ?? 0)
  const loading = server === null || program === null

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !editing) onClose() }}>
      <div className="settings-modal filters-modal" role="dialog" aria-label="Правила обработки почты">
        <div className="compose-header">
          <div><strong>Правила обработки почты</strong><small>На сервере — для каждого нового письма, даже когда программа закрыта; уведомления, задачи и условия по вложениям выполняет программа</small></div>
          <button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        {serverError && <div className="inline-warning" role="alert">Правила на сервере недоступны: {serverError}. Новые правила будет выполнять программа.</div>}
        {loading && <div className="folder-placeholder">Загрузка…</div>}
        {editing ? (
          <FilterEditor
            key={editing.item ? `${editing.item.where}${editing.item.index}` : 'new'}
            initial={editing.filter}
            folders={folders}
            categories={categories}
            serverOk={serverOk}
            label={label}
            folderName={folderName}
            busy={busy || loading}
            onCancel={() => setEditing(null)}
            onSave={(f) => void commit(f, editing.item)}
          />
        ) : !loading && <>
          <div className="filter-list">
            {items.length === 0 && <div className="folder-placeholder">Правил пока нет. Например: письма от рассылки — в отдельную папку, письма с отчётами во вложении — задачей в Deck.</div>}
            {items.map((item) => {
              const f = item.filter
              return (
                <div key={`${item.where}${item.index}`} className={`filter-item ${isActive(f) ? '' : 'off'}`} data-where={item.where}>
                  <label className="switch" title={isActive(f) ? 'Выключить' : 'Включить'}>
                    <input type="checkbox" aria-label={`Включено: ${f.name}`} checked={isActive(f)} disabled={busy} onChange={() => toggle(item)} />
                    <span />
                  </label>
                  <div className="filter-text">
                    <strong>{f.name} <span className={`rule-where ${item.where}`} title={item.where === 'server' ? 'Выполняется на сервере, даже когда программа закрыта' : 'Выполняется программой, пока Atom Workspace запущен'}>
                      {item.where === 'server' ? <><Cloud size={11} /> сервер{f.actions.some(isProgramAction) ? ' + программа' : ''}</> : <><Monitor size={11} /> программа</>}
                    </span></strong>
                    <span>{describe(f, folderName, catName)}</span>
                  </div>
                  <button className="icon-button soft" aria-label="Выше" disabled={busy || item.index === 0} onClick={() => moveItem(item, -1)}><ArrowUp size={15} /></button>
                  <button className="icon-button soft" aria-label="Ниже" disabled={busy || item.index === listLen(item.where) - 1} onClick={() => moveItem(item, 1)}><ArrowDown size={15} /></button>
                  <button className="icon-button soft" aria-label={`Изменить ${f.name}`} disabled={busy} onClick={() => setEditing({ item, filter: f })}><Pencil size={15} /></button>
                  <button className="icon-button soft danger-icon" aria-label={`Удалить ${f.name}`} disabled={busy} onClick={() => remove(item)}><Trash2 size={15} /></button>
                </div>
              )
            })}
          </div>
          <div className="settings-actions">
            <button className="primary-button" disabled={busy} onClick={() => setEditing({ item: null, filter: blank(null) })}><Plus size={15} /> Новое правило</button>
          </div>
        </>}
      </div>
    </div>
  )
}

function FilterEditor({ initial, folders, categories, serverOk, label, folderName, busy, onCancel, onSave }: {
  initial: SieveFilter
  folders: Folder[]
  categories: Category[]
  serverOk: boolean
  label: (f: Folder) => string
  folderName: (path: string) => string
  busy: boolean
  onCancel: () => void
  onSave: (f: SieveFilter) => void
}) {
  const [f, setF] = useState<SieveFilter>(() => JSON.parse(JSON.stringify(initial)) as SieveFilter)
  const [pickFor, setPickFor] = useState<number | null>(null)
  const [problem, setProblem] = useState('')
  const where = placementOf(f, serverOk)
  const needsDeck = f.actions.some((a) => a.method === 'task')
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [stacks, setStacks] = useState<Record<number, DeckStack[]>>({})
  const [deckError, setDeckError] = useState('')
  useEffect(() => {
    if (!needsDeck || boards !== null) return
    api.deckBoards().then((b) => setBoards(b.filter((x) => x.canEdit))).catch((e) => { setBoards([]); setDeckError(errorText(e)) })
  }, [needsDeck, boards])
  function loadStacks(boardId: number) {
    if (stacks[boardId]) return
    api.deckStacks(boardId).then((list) => setStacks((m) => ({ ...m, [boardId]: list }))).catch(() => undefined)
  }
  useEffect(() => {
    for (const a of f.actions) {
      const b = Number((a.argument ?? '').split(':')[0])
      if (a.method === 'task' && b) loadStacks(b)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [f.actions])

  const setRule = (i: number, patch: Partial<SieveRule>) => setF((x) => ({ ...x, rules: x.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }))
  const setAction = (i: number, patch: Partial<SieveAction>) => setF((x) => ({ ...x, actions: x.actions.map((a, j) => (j === i ? { ...a, ...patch } : a)) }))

  function submit() {
    const name = f.name.trim()
    if (!name) return setProblem('Укажите название фильтра.')
    const rules = f.match === 'allmessages' ? [] : f.rules.map((r) => ({ ...r, value: r.field === 'has_attachment' ? '1' : r.value.trim() }))
    if (f.match !== 'allmessages' && (rules.length === 0 || rules.some((r) => !r.value))) return setProblem('Заполните значения условий.')
    if (rules.some((r) => r.field === 'domain' && !/^@?[^\s@]+\.[^\s@]+$/.test(r.value))) return setProblem('Подразделение — домен адреса, например ihep.ru.')
    if (rules.some((r) => r.field === 'header' && !r.custom_header?.trim())) return setProblem('Укажите имя заголовка.')
    if (rules.some((r) => r.field === 'size' && !/^\d+$/.test(r.value))) return setProblem('Размер — целое число килобайт.')
    const actions: SieveAction[] = f.actions.map((a) => (NO_ARG.has(a.method) ? { method: a.method } : { ...a, argument: (a.argument ?? '').trim() }))
    if (actions.length === 0) return setProblem('Добавьте хотя бы одно действие.')
    if (actions.some((a) => a.method === 'fileinto' && !a.argument)) return setProblem('Выберите папку для перемещения.')
    if (actions.some((a) => a.method === 'redirect' && !/^[^\s@]+@[^\s@]+$/.test(a.argument ?? ''))) return setProblem('Укажите адрес для перенаправления.')
    if (actions.some((a) => a.method === 'category' && !a.argument)) return setProblem('Выберите категорию.')
    if (actions.some((a) => a.method === 'task' && !/^\d+:\d+$/.test(a.argument ?? ''))) return setProblem('Выберите доску и колонку Deck для задачи.')
    const placed = placementOf({ ...f, rules }, serverOk)
    if (placed === 'program' && actions.some((a) => SERVER_ONLY_ACTIONS.has(a.method))) return setProblem('Перенаправление, удаление и «оставить во Входящих» выполняет только сервер — уберите условие по вложениям или эти действия.')
    if (placed === 'program' && rules.some((r) => r.field === 'header' || r.operator.includes('regex'))) return setProblem('Условия по заголовку и регулярные выражения проверяет только сервер.')
    onSave({ ...f, name, rules, actions })
  }

  return (
    <div className="filter-editor">
      <label className="login-field"><span>Название</span><input aria-label="Название правила" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="Например: Рассылка НОЦ" /></label>
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
              const operator = field === 'size' ? 'over' : field === 'domain' || field === 'has_attachment' ? 'is' : (['size', 'domain', 'has_attachment'].includes(r.field) ? 'contains' : r.operator)
              setRule(i, { field, operator, value: field === 'has_attachment' ? '1' : r.field === 'has_attachment' ? '' : r.value })
            }}>
              {FIELDS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
            {r.field === 'header' && <input aria-label="Имя заголовка" className="filter-header" value={r.custom_header ?? ''} onChange={(e) => setRule(i, { custom_header: e.target.value })} placeholder="List-Id" />}
            {r.field !== 'has_attachment' && <>
              <select aria-label="Сравнение" value={r.operator} onChange={(e) => setRule(i, { operator: e.target.value })}>
                {(r.field === 'size' ? SIZE_OPS : r.field === 'domain' ? DOMAIN_OPS : TEXT_OPS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
              </select>
              <input aria-label="Значение" value={r.value} onChange={(e) => setRule(i, { value: e.target.value })} placeholder={r.field === 'size' ? '1024' : r.field === 'domain' ? 'ihep.ru' : r.field === 'attachment' ? '.xlsx или «отчёт»' : 'текст'} />
            </>}
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
              setAction(i, { method, argument: method === 'addflag' ? 'seen' : method === 'category' ? (categories[0]?.keyword ?? '') : method === a.method ? a.argument : '' })
            }}>
              {ACTIONS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
            {a.method === 'fileinto' && <button className="secondary-button filter-folder" onClick={() => setPickFor(i)}><FolderInput size={15} /> {a.argument ? folderName(a.argument) : 'Выбрать папку…'}</button>}
            {a.method === 'addflag' && <select aria-label="Пометка" value={a.argument ?? 'seen'} onChange={(e) => setAction(i, { argument: e.target.value })}>
              {FLAGS.map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>}
            {a.method === 'redirect' && <input aria-label="Адрес" value={a.argument ?? ''} onChange={(e) => setAction(i, { argument: e.target.value })} placeholder="ivanov@организация.ru" />}
            {a.method === 'category' && <select aria-label="Категория" value={a.argument ?? ''} onChange={(e) => setAction(i, { argument: e.target.value })}>
              {categories.length === 0 && <option value="">нет категорий</option>}
              {categories.map((c) => <option key={c.keyword} value={c.keyword}>{c.name}</option>)}
            </select>}
            {a.method === 'task' && (() => {
              const [b, st] = (a.argument ?? '').split(':').map(Number)
              return <>
                <select aria-label="Доска" value={b || ''} onChange={(e) => { const id = Number(e.target.value); loadStacks(id); setAction(i, { argument: id ? `${id}:` : '' }) }}>
                  <option value="">{boards === null ? 'Загрузка досок…' : boards.length ? 'Доска…' : 'Нет досок'}</option>
                  {(boards ?? []).map((x) => <option key={x.id} value={x.id}>{x.title}</option>)}
                </select>
                {!!b && <select aria-label="Колонка" value={st || ''} onChange={(e) => setAction(i, { argument: `${b}:${e.target.value}` })}>
                  <option value="">Колонка…</option>
                  {(stacks[b] ?? []).map((x) => <option key={x.id} value={x.id}>{x.title}</option>)}
                </select>}
              </>
            })()}
            <button className="icon-button soft" aria-label="Убрать действие" disabled={f.actions.length < 2} onClick={() => setF({ ...f, actions: f.actions.filter((_, j) => j !== i) })}><X size={15} /></button>
          </div>
        ))}
        <button className="text-button" onClick={() => setF({ ...f, actions: [...f.actions, { method: 'addflag', argument: 'seen' }] })}><Plus size={14} /> Действие</button>
      </div>
      <div className={`rule-placement ${where}`} role="note">
        {where === 'server'
          ? <><Cloud size={14} /> Выполняется на сервере — для каждого нового письма, даже когда программа закрыта.{f.actions.some(isProgramAction) && ' Уведомление и задачу добавляет программа, пока она запущена.'}</>
          : <><Monitor size={14} /> {serverOk ? 'Условие по вложениям проверяет только программа' : 'Сервер правил недоступен'} — правило выполняется, пока Atom Workspace запущен, для писем, пришедших во «Входящие».</>}
      </div>
      {deckError && needsDeck && <div className="inline-warning">Deck недоступен: {deckError}</div>}
      {problem && <div className="inline-warning" role="alert">{problem}</div>}
      <div className="settings-actions">
        <button className="secondary-button" onClick={onCancel}>Отмена</button>
        <button className="primary-button" disabled={busy} onClick={submit}>Сохранить правило</button>
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

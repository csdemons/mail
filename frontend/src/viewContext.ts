import type { MailFilter, MessageSummary, UnifiedFilters, UnifiedRole } from './types'

export type ModuleName = 'mail' | 'calendar' | 'contacts' | 'deck' | 'chat' | 'files'

/** Где пользователь был в учётной записи: модуль, папка, письмо, фильтры, прокрутка. */
export interface ViewContext {
  module: ModuleName
  folderId: string
  unified: UnifiedRole | null
  unifiedFilters: UnifiedFilters
  filter: MailFilter
  catFilter: string
  query: string
  selected: MessageSummary | null
  scrollTop: number
}

/**
 * Контекст каждой учётной записи на время работы программы: при переключении
 * обратно всё восстанавливается — без повторного поиска папки, письма и фильтров.
 * Хранится только в памяти окна (ни на диск, ни в другие окна не попадает).
 */
const contexts = new Map<string, ViewContext>()

export function savedView(account: string): ViewContext | undefined {
  return contexts.get(account)
}

export function saveView(account: string, patch: Partial<ViewContext>, base: ViewContext) {
  contexts.set(account, { ...(contexts.get(account) ?? base), ...patch })
}

/** Выход из учётной записи — её контекст забывается. */
export function forgetView(account: string) {
  contexts.delete(account)
}

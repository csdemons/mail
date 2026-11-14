import { X, Mail, ShieldCheck, ListFilter, Stethoscope, Download, LogOut, Copy } from 'lucide-react'
import { MailboxesSettings, SignaturesSettings, SendingSettings, CategoriesSettings, DesktopSettings, OfflineSettings } from './MailSettings'
import NcConnect from './NcConnect'
import { notify, type NotifyPrefs } from './notifications'
import AppearanceSettings from './AppearanceSettings'
import { TransferSettings } from './Transfer'
import { folderLabel } from './mailLabels'
import TotpSettings from './TotpSettings'
import { CHECK_UPDATE_EVENT } from './UpdateNotice'
import About from './About'
import type { Account, NcStatus, SignatureSet, Category, Folder, MessageSummary } from './types'

/** Окно «Учётная запись»: подключение, уведомления, вид, подписи, отправка, перенос, защита, диагностика. */
export interface SettingsDialogProps {
  setShowSettings: React.Dispatch<React.SetStateAction<boolean>>
  myAddress: string
  account: Account
  profileForm: { displayName: string; smtpHost: string; smtpPort: string; davUrl: string; }
  setProfileForm: React.Dispatch<React.SetStateAction<{ displayName: string; smtpHost: string; smtpPort: string; davUrl: string; }>>
  serversChanged: boolean
  confirmPassword: string
  setConfirmPassword: React.Dispatch<React.SetStateAction<string>>
  profileBusy: boolean
  saveProfile: () => Promise<void>
  loadFolders: () => Promise<void>
  setNotice: React.Dispatch<React.SetStateAction<string>>
  ncStatus: NcStatus | null
  setNcStatus: React.Dispatch<React.SetStateAction<NcStatus | null>>
  prefs: NotifyPrefs
  updatePrefs: (patch: Partial<NotifyPrefs>) => void
  notifyPolicy: "none" | "full" | "sender"
  sigs: SignatureSet | null
  setSigs: React.Dispatch<React.SetStateAction<SignatureSet | null>>
  categories: Category[]
  setCategories: React.Dispatch<React.SetStateAction<Category[]>>
  folders: Folder[]
  setTotpOn: React.Dispatch<React.SetStateAction<boolean>>
  onLock: () => void
  autolock: number
  updateAutolock: (minutes: number) => void
  setFilterSeed: React.Dispatch<React.SetStateAction<MessageSummary | null>>
  setShowFilters: React.Dispatch<React.SetStateAction<boolean>>
  diagBusy: boolean
  runDiagnostics: () => Promise<void>
  onLogout: () => void
  diagText: string | null
  copyDiagnostics: () => Promise<void>
}

export default function SettingsDialog({ setShowSettings, myAddress, account, profileForm, setProfileForm, serversChanged, confirmPassword, setConfirmPassword, profileBusy, saveProfile, loadFolders, setNotice, ncStatus, setNcStatus, prefs, updatePrefs, notifyPolicy, sigs, setSigs, categories, setCategories, folders, setTotpOn, onLock, autolock, updateAutolock, setFilterSeed, setShowFilters, diagBusy, runDiagnostics, onLogout, diagText, copyDiagnostics }: SettingsDialogProps) {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setShowSettings(false) }}>
      <div className="settings-modal">
        <div className="compose-header"><div><strong>Учётная запись</strong><small>Параметры подключения</small></div><button className="icon-button" onClick={() => setShowSettings(false)} aria-label="Закрыть"><X size={18} /></button></div>
        <div className="settings-account">
          <div className="account-avatar"><Mail size={20} /></div>
          <div><strong>{myAddress}</strong><span>{account.exchange ? `Exchange (EWS) · ${account.host}` : `IMAP · ${account.host}:${account.port}`} · TLS</span></div>
          <span className="status-pill">Подключён</span>
        </div>
        <div className="profile-form">
          <label className="login-field"><span>Имя отправителя</span><input value={profileForm.displayName} onChange={(e) => setProfileForm((f) => ({ ...f, displayName: e.target.value }))} placeholder="Юшков Дмитрий" /></label>
          <div className="login-server">
            <label className="login-field"><span>Исходящая почта (SMTP)</span><input value={profileForm.smtpHost} onChange={(e) => setProfileForm((f) => ({ ...f, smtpHost: e.target.value }))} /></label>
            <label className="login-field login-port"><span>Порт</span><input inputMode="numeric" value={profileForm.smtpPort} onChange={(e) => setProfileForm((f) => ({ ...f, smtpPort: e.target.value }))} /></label>
          </div>
          <label className="login-field"><span>Календарь и контакты (CalDAV/CardDAV)</span><input value={profileForm.davUrl} onChange={(e) => setProfileForm((f) => ({ ...f, davUrl: e.target.value }))} placeholder={`https://${account.host}/ (автоопределение)`} /></label>
          {serversChanged && <label className="login-field"><span>Пароль почты — для смены адресов серверов</span><input type="password" aria-label="Пароль для подтверждения" autoComplete="current-password" value={confirmPassword} onChange={(e) => setConfirmPassword(e.target.value)} /></label>}
          <div className="settings-actions"><button className="primary-button" disabled={profileBusy || (serversChanged && !confirmPassword)} onClick={() => void saveProfile()}>Сохранить</button></div>
        </div>
        {account.exchange && <MailboxesSettings onChanged={() => void loadFolders()} onNotice={setNotice} />}
        {ncStatus && <div className="settings-section"><strong>Nextcloud (Deck, Talk)</strong><NcConnect status={ncStatus} onChange={setNcStatus} compact /></div>}
        <div className="settings-section notify-settings">
          <strong>Уведомления</strong>
          <label><input type="checkbox" checked={prefs.mail} onChange={(e) => updatePrefs({ mail: e.target.checked })} /> О новых письмах</label>
          <label><input type="checkbox" checked={prefs.shared} disabled={!prefs.mail} onChange={(e) => updatePrefs({ shared: e.target.checked })} /> Включая подключённые (общие) ящики</label>
          <label><input type="checkbox" checked={prefs.chat} onChange={(e) => updatePrefs({ chat: e.target.checked })} /> О сообщениях в чате</label>
          <label><input type="checkbox" checked={prefs.preview && notifyPolicy === 'full'} disabled={notifyPolicy !== 'full'} onChange={(e) => updatePrefs({ preview: e.target.checked })} /> Показывать тему письма и текст сообщения</label>
          {notifyPolicy !== 'full' && <span className="settings-hint">{notifyPolicy === 'none' ? 'Администратор разрешил показывать в уведомлениях только сам факт нового письма или сообщения.' : 'Администратор запретил показывать в уведомлениях тему и текст.'}</span>}
          <button className="text-button" onClick={() => notify('Atom Workspace', 'Так выглядят уведомления о новой почте и сообщениях')}>Показать пример уведомления</button>
        </div>
        <AppearanceSettings />
        <SignaturesSettings set={sigs} onSaved={(set, msg) => { setSigs(set); setNotice(msg) }} />
        <SendingSettings />
        <CategoriesSettings categories={categories} onSaved={(list, msg) => { setCategories(list); setNotice(msg) }} />
        <DesktopSettings onNotice={setNotice} />
        <OfflineSettings onNotice={setNotice} />
        <TransferSettings folders={folders} label={folderLabel} onNotice={setNotice} onImported={() => void loadFolders()} />
        <TotpSettings onChange={setTotpOn} onLock={() => { setShowSettings(false); onLock() }} autolock={autolock} onAutolock={updateAutolock} />
        <div className="settings-note"><ShieldCheck size={18} /><span>Пароль хранится только в системном хранилище (Windows Credential Manager, GNOME Keyring, Связка ключей macOS) и не записывается в файлы настроек. Для SMTP, календаря и контактов используются те же логин и пароль.</span></div>
        <div className="settings-actions">
          <button className="secondary-button" onClick={() => { setShowSettings(false); setFilterSeed(null); setShowFilters(true) }}><ListFilter size={16} /> Правила обработки почты</button>
          <button className="secondary-button" disabled={diagBusy} onClick={() => void runDiagnostics()}><Stethoscope size={16} /> Диагностика</button>
          <button className="secondary-button" onClick={() => window.dispatchEvent(new Event(CHECK_UPDATE_EVENT))}><Download size={16} /> Проверить обновления</button>
          <button className="secondary-button" onClick={() => { setShowSettings(false); onLogout() }}><LogOut size={16} /> Выйти и забыть пароль</button>
        </div>
        <About />
        {diagText !== null && <div className="diag-block">
          <div className="diag-head"><span>Отчёт: IMAP по текущей папке, SMTP, CalDAV/CardDAV (пароль не включается)</span><button className="text-button" onClick={() => void copyDiagnostics()}><Copy size={14} /> Скопировать</button></div>
          <textarea className="diag-text" readOnly value={diagText} />
        </div>}
      </div>
    </div>
  )
}

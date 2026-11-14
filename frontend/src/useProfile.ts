import { useEffect, useState } from 'react'

import { api, errorText } from './api'
import type { Settings as Profile } from './types'

/** Профиль учётной записи (имя отправителя, SMTP, CalDAV) и форма его правки в настройках. */
export function useProfile(setNotice: (text: string) => void) {
  const [profile, setProfile] = useState<Profile | null>(null)
  const [profileForm, setProfileForm] = useState({ displayName: '', smtpHost: '', smtpPort: '465', davUrl: '' })
  const [confirmPassword, setConfirmPassword] = useState('')
  const serversChanged = Boolean(profile) && (profileForm.smtpHost.trim() !== profile?.smtpHost || profileForm.smtpPort.trim() !== String(profile?.smtpPort) || profileForm.davUrl.trim() !== profile?.davUrl)
  const [profileBusy, setProfileBusy] = useState(false)

  useEffect(() => {
    api.getProfile().then((p) => {
      setProfile(p)
      if (p) setProfileForm({ displayName: p.displayName, smtpHost: p.smtpHost, smtpPort: String(p.smtpPort), davUrl: p.davUrl })
    }).catch(() => undefined)
  }, [])

  async function saveProfile() {
    const port = Number(profileForm.smtpPort)
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setNotice('Некорректный порт SMTP')
      return
    }
    setProfileBusy(true)
    try {
      const p = await api.updateProfile({ ...profileForm, smtpPort: port, password: serversChanged ? confirmPassword : null })
      setProfile(p)
      setConfirmPassword('')
      setNotice('Настройки сохранены.')
    } catch (error) {
      setNotice(`Не удалось сохранить настройки: ${errorText(error)}`)
    } finally {
      setProfileBusy(false)
    }
  }

  return { profile, profileForm, setProfileForm, confirmPassword, setConfirmPassword, serversChanged, profileBusy, saveProfile }
}

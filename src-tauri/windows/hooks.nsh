; Каталог политик администратора %ProgramData%\RRCKI Mail (update.json, policy.json).
; При установке с правами администратора: владелец — Администраторы, запись — только
; Администраторы и SYSTEM, пользователи — только чтение. Иначе любой пользователь мог бы
; создать этот каталог заранее и отключить или перенаправить обновления для всех.
!macro NSIS_HOOK_POSTINSTALL
  UserInfo::GetAccountType
  Pop $0
  StrCmp $0 "Admin" 0 rrcki_policy_done
    ReadEnvStr $1 PROGRAMDATA
    CreateDirectory "$1\RRCKI Mail"
    nsExec::Exec 'icacls "$1\RRCKI Mail" /setowner *S-1-5-32-544 /T /C'
    Pop $2
    nsExec::Exec 'icacls "$1\RRCKI Mail" /inheritance:r /grant:r *S-1-5-32-544:(OI)(CI)F *S-1-5-18:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /T /C'
    Pop $2
  rrcki_policy_done:
!macroend

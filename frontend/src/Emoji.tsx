import { useState } from 'react'

/** Набор эмодзи без внешних библиотек: частые — в начале, дальше по группам. */
export const QUICK_REACTIONS = ['👍', '❤️', '😂', '😮', '😢', '🎉', '👀', '✅']

const GROUPS: [string, string][] = [
  ['Смайлы', '😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 😉 😊 😇 🥰 😍 🤩 😘 😋 😛 😜 🤪 🤔 🤨 😐 😑 😶 🙄 😏 😬 😌 😔 😪 😴 😷 🤒 🤕 🤧 🥵 🥶 😵 🤯 🤠 🥳 😎 🤓 🧐 😕 😟 🙁 😮 😯 😲 😳 🥺 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬'],
  ['Жесты', '👍 👎 👌 ✌️ 🤞 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ ✋ 🤚 🖐 🖖 👋 🤝 🙏 👏 🙌 👐 💪 ✍️ 🫡 🤷 🙋 🙅 🙆 🤦'],
  ['Работа', '✅ ☑️ ❌ ❗ ❓ ⚠️ 🔥 ⭐ 🌟 💡 📌 📎 📝 📅 📆 ⏰ ⌛ 🗂 📁 📂 📊 📈 📉 📋 🖥 💻 ⌨️ 🖨 📱 ☎️ 📞 📧 ✉️ 📨 🔒 🔓 🔑 🛠 🔧 🔨 ⚙️ 🧰 🧪 🚀 🏁 🎯 💼 🏢'],
  ['Разное', '❤️ 🧡 💛 💚 💙 💜 🖤 🤍 💔 💯 🎉 🎊 🎁 🎂 ☕ 🍵 🍰 🍕 🍎 🌞 🌧 ❄️ ⛄ 🌈 🌸 🌲 🐱 🐶 👀 💤 💬 👋 🙂'],
]

export function EmojiPicker({ onPick }: { onPick: (emoji: string) => void }) {
  const [group, setGroup] = useState(0)
  return (
    <div className="emoji-picker popover" onMouseDown={(e) => e.preventDefault()}>
      <div className="emoji-tabs">
        {GROUPS.map(([name], i) => (
          <button type="button" key={name} className={i === group ? 'active' : ''} onClick={() => setGroup(i)}>{name}</button>
        ))}
      </div>
      <div className="emoji-grid">
        {GROUPS[group][1].split(' ').map((e) => (
          <button type="button" key={e} aria-label={e} onClick={() => onPick(e)}>{e}</button>
        ))}
      </div>
    </div>
  )
}

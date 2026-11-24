import type { TalkRoom } from './types'

export interface TalkRoomSeen {
  messageId: number
  activity: number
  unread: number
  lastRead: number
}

/** Вызывать только для серверных снимков, до локального обнуления счётчика. */
export function observeTalkRoom(previous: TalkRoomSeen | undefined, room: TalkRoom) {
  const messageId = room.lastMessageId || 0
  // Запоздалый снимок не опускает уже увиденную границу и не вызывает повторного уведомления.
  if (previous && (room.lastActivity < previous.activity || (messageId > 0 && messageId < previous.messageId))) {
    return { seen: previous, isNew: false }
  }
  const seen: TalkRoomSeen = {
    messageId: Math.max(previous?.messageId ?? 0, messageId),
    activity: Math.max(previous?.activity ?? 0, room.lastActivity),
    unread: room.unread,
    lastRead: room.lastRead,
  }
  if (!previous) return { seen, isNew: false }
  // В конфиденциальных беседах сервер может не вернуть lastMessage. Сравниваем сырые
  // счётчики, включая несколько сообщений за секунду; ручное «непрочитано» не уведомляет.
  const isNew = messageId > 0
    ? messageId > previous.messageId
    : room.lastRead >= previous.lastRead && (room.lastActivity > previous.activity || room.unread > previous.unread)
  return { seen, isNew }
}

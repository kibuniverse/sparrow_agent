import { createLazyFileRoute } from '@tanstack/react-router'
import { ChatPage } from '../pages/ChatPage'

export const Route = createLazyFileRoute('/')({
  component: ChatPage,
})

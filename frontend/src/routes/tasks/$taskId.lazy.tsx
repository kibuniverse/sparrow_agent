import { createLazyFileRoute } from '@tanstack/react-router'
import { TaskDetailPage } from '../../pages/TaskDetailPage'

function TaskDetailRoute() {
  const { taskId } = Route.useParams()
  return <TaskDetailPage taskId={taskId} />
}

export const Route = createLazyFileRoute('/tasks/$taskId')({
  component: TaskDetailRoute,
})

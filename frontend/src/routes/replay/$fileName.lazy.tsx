import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceReplayPage } from '../../pages/TraceReplayPage'

function TraceReplayRoute() {
  const { fileName } = Route.useParams()
  return <TraceReplayPage fileName={decodeURIComponent(fileName)} />
}

export const Route = createLazyFileRoute('/replay/$fileName')({
  component: TraceReplayRoute,
})

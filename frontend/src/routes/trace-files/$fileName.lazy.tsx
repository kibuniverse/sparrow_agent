import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceArchivePage } from '../../pages/TraceArchivePage'

function TraceArchiveRoute() {
  const { fileName } = Route.useParams()
  return <TraceArchivePage fileName={decodeURIComponent(fileName)} />
}

export const Route = createLazyFileRoute('/trace-files/$fileName')({
  component: TraceArchiveRoute,
})

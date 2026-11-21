import { Link, Page, PageHeader, Text } from '../ui';

export function NotFoundPage() {
  return (
    <Page>
      <PageHeader title="Page not found" />
      <Text>
        <Link to="/workflows">Go to the workflows</Link>
      </Text>
    </Page>
  );
}

import { MOQtailClient } from 'moqtail/client';

export async function createMoqtailClient(relayUrl?: string): Promise<MOQtailClient> {
  return await MOQtailClient.new({
    url: relayUrl ?? window.appSettings.relayUrl,
    transportOptions: {},
    enableDatagrams: false,
  });
}

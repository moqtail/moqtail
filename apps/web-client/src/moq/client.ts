import { MOQtailClient } from 'moqtail-ts/client'

export async function createMoqtailClient(): Promise<MOQtailClient> {
  return await MOQtailClient.new({
    url: window.appSettings.relayUrl,
    transportOptions: {},
    enableDatagrams: false,
  })
}

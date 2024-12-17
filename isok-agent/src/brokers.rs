use isok_data::Event;
use tokio::sync::RwLock;

enum Broker {
    Grpc,
    Dev(DevBroker),
}

#[async_trait::async_trait]
impl SendEvents for Broker {
    async fn send_events(&self, event: Event) -> crate::Result<()> {
        match self {
            Broker::Grpc => Ok(()),
            Broker::Dev(broker) => broker.send_events(event).await,
        }
    }
}

#[async_trait::async_trait]
trait SendEvents {
    async fn send_events(&self, event: Event) -> crate::Result<()>;
}

struct DevBroker {
    events: RwLock<Vec<Event>>,
}

#[async_trait::async_trait]
impl SendEvents for DevBroker {
    async fn send_events(&self, event: Event) -> crate::Result<()> {
        self.events.write().await.push(event);

        Ok(())
    }
}
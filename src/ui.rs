use crate::AppUpdate;
use std::sync::mpsc::Sender;
use tracing_subscriber::Layer;

// Custom tracing layer that sends logs to the UI
pub struct UiLayer {
    pub tx: Sender<AppUpdate>,
}

impl<S> Layer<S> for UiLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use tracing::field::Visit;

        struct MessageVisitor {
            message: String,
        }

        impl Visit for MessageVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = format!("{:?}", value);
                    // Remove surrounding quotes from debug format
                    if self.message.starts_with('"') && self.message.ends_with('"') {
                        self.message = self.message[1..self.message.len() - 1].to_string();
                    }
                }
            }
        }

        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);

        if !visitor.message.is_empty() {
            let level = event.metadata().level();
            let log_msg = format!("[{}] {}", level, visitor.message);
            let _ = self.tx.send(AppUpdate::Log(log_msg));
        }
    }
}

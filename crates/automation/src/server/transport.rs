//! Recover malformed JSON lines before passing valid messages to rmcp.
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

impl ToolboxMcp {
    /// Serve a JSON-lines stream. A malformed line receives -32700/id:null; the next line is
    /// still handled. A single writer serializes parse errors and SDK responses.
    pub async fn serve_io(
        self,
        input: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        output: impl tokio::io::AsyncWrite + Unpin + Send + 'static,
    ) -> Result<(), AutomationError> {
        let (mut to_service, service_in) = tokio::io::duplex(1 << 20);
        let (service_out, from_service) = tokio::io::duplex(1 << 20);
        let (errors, mut error_rx) = tokio::sync::mpsc::channel::<String>(16);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(input).lines();
            while let Some(line) = lines.next_line().await? {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(_) => to_service.write_all(format!("{line}\n").as_bytes()).await?,
                    Err(e) => {
                        let reply = json!({"jsonrpc":"2.0", "id":null, "error":{"code":-32700, "message":format!("parse error: {e}")}});
                        if errors.send(reply.to_string()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            to_service.shutdown().await
        });
        let writer = tokio::spawn(async move {
            let mut output = output;
            let mut replies = BufReader::new(from_service).lines();
            loop {
                let line = tokio::select! {
                    biased;
                    Some(error) = error_rx.recv() => error,
                    reply = replies.next_line() => match reply? { Some(line) => line, None => break },
                };
                output.write_all(format!("{line}\n").as_bytes()).await?;
                output.flush().await?;
            }
            Ok::<_, std::io::Error>(())
        });
        let result = match self.serve((service_in, service_out)).await {
            Ok(running) => running.waiting().await.map(|_| ()).map_err(|e| AutomationError::Other(e.to_string())),
            Err(e) => Err(AutomationError::Other(format!("MCP init: {e}"))),
        };
        reader.abort();
        match reader.await {
            Ok(r) => r.map_err(|e| AutomationError::Io(e.to_string()))?,
            Err(e) if !e.is_cancelled() => return Err(AutomationError::Other(e.to_string())),
            Err(_) => {}
        }
        writer.await.map_err(|e| AutomationError::Other(e.to_string()))?.map_err(|e| AutomationError::Io(e.to_string()))?;
        result
    }
}

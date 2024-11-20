use tokio::net::TcpStream;
use tokio::io::{self,AsyncReadExt};
use tokio::sync::{mpsc,watch};
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReceiverError {
    #[error("Failed to connect to server: {0}")]
    ConnectionError(std::io::Error),
    #[error("Failed to receive data: {0}")]
    ReceiveError(std::io::Error),
}

// Define the type for the frame data (adjust as needed for your use case)
type Frame = Vec<u8>;

// Type alias for the frame receiver
pub type FrameReceiver = mpsc::Receiver<Frame>;

// The function to connect to the server and start receiving frames
#[derive(Clone)]
pub struct DisconnectHandle {
    shutdown_tx: watch::Sender<bool>,
}

impl DisconnectHandle {
    pub async fn disconnect(self) {
        // Signal the background task to stop
        let _ = self.shutdown_tx.send(true);
    }
}

// The function to connect to the server and start receiving frames
pub async fn connect_to_server(
    ip_address: &str,
) -> Result<(mpsc::Receiver<Vec<u8>>, DisconnectHandle), ReceiverError> {
    let port = 9041;
    let address_port = format!("{}:{}", ip_address, port);

    let addr: SocketAddr = address_port.parse().map_err(|e| {
        ReceiverError::ConnectionError(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e,
        ))
    })?;

    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(ReceiverError::ConnectionError)?;

    println!("Successfully connected to {}", addr);

    // Create an MPSC channel to send frames from the receiver task
    let (frame_tx, frame_rx) = mpsc::channel(10);

    // Create a watch channel for shutdown signaling
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    // Spawn a task to handle receiving data from the server
    tokio::spawn(async move {
        let mut buffer = vec![0; 100000000]; // Adjust buffer size as needed

        loop {
            // Check for shutdown signal
            if *shutdown_rx.borrow() {
                break;
            }

            let mut size_buffer = [0u8; 4];
            match stream.read_exact(&mut size_buffer).await {
                Ok(_) => {
                    let frame_size = u32::from_be_bytes(size_buffer) as usize;

                    // Step 2: Read the frame data
                    let mut frame_buffer = vec![0u8; frame_size];
                    match stream.read_exact(&mut frame_buffer).await {
                        Ok(_) => {
                            // Step 3: Send the frame to the main application via the channel
                            if frame_tx.send(frame_buffer).await.is_err() {
                                // If the receiver side is closed, stop the loop
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to read frame data: {}", e);
                            // Handle EOF or other read errors
                            if e.kind() == io::ErrorKind::UnexpectedEof {
                                println!("Connection closed by server.");
                                if let Err(_e) = frame_tx.send(vec![]).await {
                                    eprintln!("Failed to notify receiver about connection closure");
                                }
                            }
                            break;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to read frame size: {}", e);
                    // Handle EOF or other read errors
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        println!("Connection closed by server.");
                        if let Err(_e) = frame_tx.send(vec![]).await {
                            eprintln!("Failed to notify receiver about connection closure");
                        }
                    }
                    break;
                }
            }
        }
        if let Ok(stream_std) = stream.into_std() {
            use std::net::Shutdown;
            if let Err(e) = stream_std.shutdown(Shutdown::Both) {
                eprintln!("Error shutting down the connection: {}", e);
            }
            drop(stream_std);
        }
        println!("Receiver task exiting.");
    });

    // Return the frame receiver and disconnect handle to the caller
    let disconnect_handle = DisconnectHandle { shutdown_tx };
    Ok((frame_rx, disconnect_handle))
}
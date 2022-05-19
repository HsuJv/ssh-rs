use std::io;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::Ordering::Relaxed;
use constant::{ssh_msg_code, size};
use encryption::{ChaCha20Poly1305, IS_ENCRYPT};
use packet::{Data, Packet};
use error::{SshError, SshResult};

use crate::channel::ChannelWindowSize;
use crate::util;





pub struct Client {
    pub(crate) stream: TcpStream,
    sequence: Sequence,
}

#[derive(Clone)]
struct Sequence {
    client_sequence_num: u32,
    server_sequence_num: u32
}

impl Sequence {

    fn client_auto_increment(&mut self) {
        if self.client_sequence_num == u32::MAX {
            self.client_sequence_num = 0;
        }
        self.client_sequence_num += 1;
    }

    fn server_auto_increment(&mut self) {
        if self.server_sequence_num == u32::MAX {
            self.server_sequence_num = 0;
        }
        self.server_sequence_num += 1;
    }
}


impl Client {
    pub fn connect<A: ToSocketAddrs>(adder: A) -> Result<Client, SshError> {
        match TcpStream::connect(adder) {
            Ok(stream) =>
                Ok(
                    Client{
                        stream,
                        sequence: Sequence {
                            client_sequence_num: 0,
                            server_sequence_num: 0
                        },
                    }
                ),
            Err(e) => Err(SshError::from(e))
        }

    }

    pub fn read_version(&mut self) -> Vec<u8>  {
        let mut v = [0_u8; 128];
        loop {
            match self.stream.read(&mut v) {
                Ok(i) => { return (&v[..i]).to_vec() }
                Err(_) => continue
            };
        }
    }

    pub(crate) fn read(&mut self) -> Result<Vec<Data>, SshError> {
        let mut results = vec![];
        let mut result = vec![0; size::BUF_SIZE as usize];
        let len = match self.stream.read(&mut result) {
            Ok(len) => {
                if len <= 0 {
                    return Ok(results)
                }
                len
            },
            Err(e) => {
                if is_would_block(&e) {
                    return Ok(results)
                }
                return Err(SshError::from(e))
            }
        };
        result.truncate(len);
        self.process_data(result, &mut results)?;
        Ok(results)
    }

    pub fn write_version(&mut self, buf: &[u8]) -> Result<(), SshError> {
        match self.stream.write(&buf) {
            Ok(_) => Ok(()),
            Err(e) => Err(SshError::from(e))
        }
    }

    pub fn write(&mut self, buf: Data) -> Result<(), SshError> {
        //let mut packet = Packet::from(buf.to_vec());
        //let mut data = packet.unpacking();

        // let mut data = Data(buf.to_vec());
        // println!("数据包总长度 {}", data.get_u32());
        // println!("填充长度 {}", data.get_u8());
        // println!("消息标志 {:?}", data.get_u8());

        // TODO 暂时不使用
        // let mut data = buf.clone();
        //
        // let msg_code = data.get_u8();
        //
        // let (client_channel_no, size, flag) = match msg_code {
        //     ssh_msg_code::SSH_MSG_CHANNEL_DATA => {
        //         let client_channel_no = data.get_u32(); // channel serial no    4 len
        //         let vec = data.get_u8s(); // string data len
        //         let size = vec.len() as u32;
        //         (client_channel_no, size, true)
        //     }
        //     ssh_msg_code::SSH_MSG_CHANNEL_EXTENDED_DATA => {
        //         let client_channel_no = data.get_u32(); // channel serial no    4 len
        //         data.get_u32(); // data type code        4 len
        //         let vec = data.get_u8s();  // string data len
        //         let size = vec.len() as u32;
        //         (client_channel_no, size, true)
        //     }
        //     _ => (0, 0, false)
        // };

        // if flag {
        //
        //     let result = util::get_channel_window(client_channel_no).unwrap();
        //
        //     if let Some(mut v) = result {
        //
        //         let s = size::LOCAL_WINDOW_SIZE - v.r_window_size;
        //         println!("s => {}", s);
        //         if v.r_window_size > 0 && s > 0 && size::LOCAL_WINDOW_SIZE / s <= 20 {
        //             println!("已使用20分之一");
        //             'main:
        //             loop {
        //                 let datas = self.read().unwrap();
        //                 if !datas.is_empty() {
        //                     for mut x in datas {
        //                         let mc = x.get_u8();
        //                         println!("消息码: {}", mc);
        //                         if ssh_msg_code::SSH_MSG_CHANNEL_WINDOW_ADJUST == mc {
        //                             println!("SSH_MSG_CHANNEL_WINDOW_ADJUST");
        //                             let c = x.get_u32();
        //                             println!("通道编号: {}", c);
        //                             let i = x.get_u32();
        //                             println!("远程客户端大小: {}", i);
        //                             v.r_window_size = v.r_window_size + i;
        //                             break 'main;
        //                         }
        //                     }
        //                 }
        //             }
        //         }
        //
        //         v.r_window_size = v.r_window_size - size;
        //
        //         println!("r_window_size: {}", v.r_window_size);
        //     }
        //
        // }

        let mut packet = Packet::from(buf);
        let buf = if IS_ENCRYPT.load(Relaxed) {
            packet.build(true);
            let mut buf = packet.to_vec();
            let key = util::encryption_key()?;
            key.encryption(self.sequence.client_sequence_num, &mut buf);
            buf
        } else {
            packet.build(false);
            packet.to_vec()
        };

        self.sequence.client_auto_increment();

        if let Err(e) = self.stream.write(&buf) {
            return Err(SshError::from(e))
        }

        if let Err(e) = self.stream.flush() {
            return Err(SshError::from(e))
        }

        Ok(())
    }

    pub(crate) fn close(&mut self) -> Result<(), SshError> {
        match self.stream.shutdown(Shutdown::Both) {
            Ok(o) => Ok(o),
            Err(e) => Err(SshError::from(e))
        }
    }

    fn process_data(&mut self, mut result: Vec<u8>, results: &mut Vec<Data>) -> SshResult<()> {
        // 未加密
        if !IS_ENCRYPT.load(Relaxed) {
            self.sequence.server_auto_increment();
            let packet_len = &result[..4];
            let mut packet_len_slice = [0_u8; 4];
            packet_len_slice.copy_from_slice(packet_len);
            let packet_len = (u32::from_be_bytes(packet_len_slice) as usize) + 4;
            // 唯一处理 server Key Exchange Reply 和 New Keys 会一块发
            if result.len() > packet_len {
                let (v1, v2) = result.split_at_mut(packet_len);
                let data = Packet::from(v1.to_vec()).unpacking();
                results.push(data);
                result = v2.to_vec();
            }
            let data = Packet::from(result).unpacking();
            results.push(data);
            return Ok(())
        }

        // 加密数据
        self.process_data_encrypt(result, results)

    }


    fn process_data_encrypt(&mut self, mut result: Vec<u8>, results: &mut Vec<Data>) -> SshResult<()> {
        loop {
            self.sequence.server_auto_increment();
            if result.len() < 4 {
                self.check_result_len(&mut result)?;
            }
            let key = util::encryption_key()?;
            let packet_len = self.get_encrypt_packet_length(&result[..4], key);
            let data_len = (packet_len + 4 + 16) as usize;
            if result.len() < data_len {
                self.get_encrypt_data(&mut result, data_len)?;
            }
            let (this, remaining) = result.split_at_mut(data_len);
            let decryption_result =
                key.decryption(self.sequence.server_sequence_num, &mut this.to_vec())?;
            let data = Packet::from(decryption_result).unpacking();

            // change the channel window size
            ChannelWindowSize::process_window_size(data.clone(), self)?;

            results.push(data);

            if remaining.len() <= 0 {
                break;
            }

            result = remaining.to_vec();
        }
        Ok(())
    }

    fn get_encrypt_data(&mut self, result: &mut Vec<u8>, data_len: usize) -> SshResult<()> {
        loop {
            let mut buf = vec![0; size::BUF_SIZE as usize];
            match self.stream.read(&mut buf) {
                Ok(len) => {
                    if len > 0 {
                        buf.truncate(len);
                        result.extend(buf);
                    }
                    if result.len() >= data_len {
                        return Ok(())
                    }
                },
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        continue;
                    }
                    return Err(SshError::from(e))
                }
            };
        }
    }

    fn get_encrypt_packet_length(&self, len: &[u8], key: &mut ChaCha20Poly1305) -> u32 {
        let mut packet_len_slice = [0_u8; 4];
        packet_len_slice.copy_from_slice(len);
        let packet_len_slice = key.server_key
            .decrypt_packet_length(
                self.sequence.server_sequence_num,
                packet_len_slice);
        u32::from_be_bytes(packet_len_slice)
    }

    fn check_result_len(&mut self, result: &mut Vec<u8>) -> SshResult<usize> {
        loop {
            let mut buf = vec![0; size::BUF_SIZE as usize];
            match self.stream.read(&mut buf) {
                Ok(len) => {
                    buf.truncate(len);
                    result.extend(buf);
                    if result.len() >= 4 {
                        return Ok(len)
                    }
                },
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        continue;
                    }
                    return Err(SshError::from(e))
                }
            };
        }
    }
}

fn is_would_block(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::WouldBlock
}

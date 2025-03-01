use bollard::Docker;
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    async fn new(remote_url: Option<String>) -> anyhow::Result<Self> {
        let docker = match remote_url {
            Some(url) => {
                // 普通 HTTP 远程连接
                Docker::connect_with_http(&url, 120, bollard::API_DEFAULT_VERSION)
                    .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))?
            }
            None => {
                // 本地连接 (自动检测平台: Windows, Linux, macOS)
                Docker::connect_with_local_defaults()
                    .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))?
            }
        };
        Ok(Self { docker })
    }

    /// 格式化字节大小
    fn format_bytes(bytes: u64) -> String {
        const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        format!("{:.2} {}", size, UNITS[unit_index])
    }

    async fn ensure_image_exists(&self, image_name: &str) -> anyhow::Result<()> {
        // 检查镜像是否存在
        if self.docker.inspect_image(image_name).await.is_err() {
            println!("镜像不存在，开始拉取 {}", image_name);
            let start_time = Instant::now();
            let mut last_print = Instant::now();

            let options = Some(CreateImageOptions {
                from_image: image_name.to_string(),
                ..Default::default()
            });

            let mut pull_stream = self.docker.create_image(options, None, None);
            while let Some(result) = pull_stream.next().await {
                match result {
                    Ok(info) => {
                        // 每秒最多打印一次进度
                        if last_print.elapsed().as_secs() >= 1 {
                            if let Some(status) = &info.status {
                                if let Some(progress) = &info.progress {
                                    println!("[{}] {}", status, progress);
                                } else {
                                    println!("[{}]", status);
                                }
                            }
                            last_print = Instant::now();
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            println!(
                "镜像拉取完成，耗时 {:.2} 秒",
                start_time.elapsed().as_secs_f32()
            );
        }
        Ok(())
    }

    async fn export_image(&self, image_name: &str, output_path: &str) -> anyhow::Result<()> {
        // 确保镜像存在
        self.ensure_image_exists(image_name).await?;

        // 获取镜像大小
        let image_info = self.docker.inspect_image(image_name).await?;
        let total_size = image_info.size.unwrap_or(0) as u64;

        // 确保输出目录存在
        if let Some(parent) = Path::new(output_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 创建输出文件
        let mut file = File::create(output_path)?;
        println!(
            "开始导出镜像 {} (总大小: {}) 到 {}",
            image_name,
            Self::format_bytes(total_size),
            output_path
        );

        // 获取导出流
        let mut export_stream = self.docker.export_image(image_name);
        let mut total_bytes = 0;
        let start_time = Instant::now();
        let mut last_print = Instant::now();

        // 处理数据流并写入文件
        while let Some(chunk) = export_stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)?;
            total_bytes += chunk.len();

            // 每秒最多打印一次进度
            if last_print.elapsed().as_secs() >= 1 {
                let percentage = if total_size > 0 {
                    (total_bytes as f64 / total_size as f64 * 100.0).min(100.0)
                } else {
                    0.0
                };
                println!(
                    "已导出: {} / {} ({:.1}%)",
                    Self::format_bytes(total_bytes as u64),
                    Self::format_bytes(total_size),
                    percentage
                );
                last_print = Instant::now();
            }
        }

        let duration = start_time.elapsed().as_secs_f32();
        let speed = total_bytes as f32 / duration / 1024.0 / 1024.0;
        println!(
            "镜像导出完成，大小: {}，耗时: {:.1} 秒，平均速度: {:.1} MB/s",
            Self::format_bytes(total_bytes as u64),
            duration,
            speed
        );
        Ok(())
    }
}

/// 主函数 - 处理输入参数并执行导出
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 获取命令行参数
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        println!("用法: {} <镜像名称> <输出路径> [远程Docker地址]", args[0]);
        println!("示例 (本地导出): {} nginx:latest ./nginx.tar", args[0]);
        println!(
            "示例 (远程导出): {} nginx:latest ./nginx.tar tcp://192.168.1.10:2375",
            args[0]
        );
        return Ok(());
    }

    let image_name = &args[1];
    let output_path = &args[2];
    let remote_url = args.get(3).map(|s| s.to_string());

    // 创建 Docker 客户端
    let docker_client = DockerClient::new(remote_url).await?;

    // 导出镜像
    docker_client.export_image(image_name, output_path).await?;

    Ok(())
}

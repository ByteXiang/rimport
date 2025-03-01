use bollard::Docker;
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

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

/// 创建 Docker 客户端，支持本地和远程连接
async fn create_docker_client(remote_url: Option<String>) -> Result<Docker, Box<dyn Error>> {
    match remote_url {
        Some(url) => {
            // 普通 HTTP 远程连接
            Docker::connect_with_http(&url, 120, bollard::API_DEFAULT_VERSION).map_err(|e| e.into())
        }
        None => {
            // 本地连接 (自动检测平台: Windows, Linux, macOS)
            Docker::connect_with_local_defaults().map_err(|e| e.into())
        }
    }
}

/// 确保镜像存在，如果不存在则拉取
async fn ensure_image_exists(docker: &Docker, image_name: &str) -> Result<(), Box<dyn Error>> {
    // 检查镜像是否存在
    if docker.inspect_image(image_name).await.is_err() {
        println!("镜像不存在，开始拉取 {}", image_name);
        let start_time = Instant::now();
        let mut last_print = Instant::now();

        let options = Some(CreateImageOptions {
            from_image: image_name.to_string(),
            ..Default::default()
        });

        let mut pull_stream = docker.create_image(options, None, None);
        while let Some(result) = pull_stream.next().await {
            match result {
                Ok(info) => {
                    // 每秒最多打印一次进度
                    if last_print.elapsed().as_secs() >= 1 {
                        println!("拉取进度: {:?}", info);
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

/// 导出 Docker 镜像到本地文件
async fn export_image(
    docker_client: &Docker,
    image_name: &str,
    output_path: &str,
) -> Result<(), Box<dyn Error>> {
    // 确保镜像存在
    ensure_image_exists(docker_client, image_name).await?;

    // 确保输出目录存在
    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 创建输出文件
    let mut file = File::create(output_path)?;
    println!("开始导出镜像 {} 到 {}", image_name, output_path);

    // 获取导出流
    let mut export_stream = docker_client.export_image(image_name);
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
            println!(
                "已导出: {} ({:.1}%)",
                format_bytes(total_bytes as u64),
                total_bytes as f64 / 1024.0 / 1024.0
            );
            last_print = Instant::now();
        }
    }

    let duration = start_time.elapsed().as_secs_f32();
    let speed = total_bytes as f32 / duration / 1024.0 / 1024.0;
    println!(
        "镜像导出完成，大小: {}，耗时: {:.1} 秒，平均速度: {:.1} MB/s",
        format_bytes(total_bytes as u64),
        duration,
        speed
    );
    Ok(())
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
    let docker = create_docker_client(remote_url.clone()).await?;

    // 显示连接信息
    if let Some(url) = remote_url {
        println!("已连接到远程 Docker: {}", url);
    } else {
        println!("已连接到本地 Docker");
    }

    // 导出镜像
    export_image(&docker, image_name, output_path).await?;

    Ok(())
}

//! compare_jxlの測定結果と画質比較画像を、単独で開けるHTMLにまとめる。
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Rgb, RgbImage, imageops};
use serde_json::{Value, json};
use std::{fs, path::Path};
fn number(row: &Value, key: &str) -> f64 {
    row[key].as_f64().unwrap_or(0.0)
}
fn mosaic(
    root: &Path,
    region: &str,
    layer: &str,
    z: u32,
    kind: &str,
    mask_distance: &str,
) -> Result<GrayImage> {
    let n = 1 << (z - 10);
    let dir = root.join(format!("{region}-{layer}"));
    if kind == "grouped" {
        return Ok(image::open(dir.join(format!(
            "progressive-d{}-z{z}.png",
            if layer == "hillshade" {
                "1"
            } else {
                mask_distance
            }
        )))?
        .to_luma8());
    }
    let mut image = GrayImage::new(384 * n, 384 * n);
    for dy in 0..n {
        for dx in 0..n {
            let filename = if kind == "current" {
                format!("current-{z}-{dx}-{dy}.png")
            } else {
                format!(
                    "independent-d{}-{z}-{dx}-{dy}.png",
                    if layer == "hillshade" {
                        "1"
                    } else {
                        mask_distance
                    }
                )
            };
            imageops::replace(
                &mut image,
                &image::open(dir.join(filename))?.to_luma8(),
                (384 * dx) as i64,
                (384 * dy) as i64,
            );
        }
    }
    Ok(image)
}
fn color(root: &Path, region: &str, z: u32, kind: &str, mask_distance: &str) -> Result<RgbImage> {
    let forest = mosaic(root, region, "forest", z, kind, mask_distance)?;
    let water = mosaic(root, region, "water", z, kind, mask_distance)?;
    let shade = mosaic(root, region, "hillshade", z, kind, mask_distance)?;
    let mut out = RgbImage::new(forest.width(), forest.height());
    for (x, y, p) in out.enumerate_pixels_mut() {
        let f = forest[(x, y)][0] as f64 / 255.0;
        let w = water[(x, y)][0] as f64 / 255.0;
        let gain = 1.0 + 0.65 * (shade[(x, y)][0] as f64 - 136.0) / 136.0;
        let mut c = [0; 3];
        for i in 0..3 {
            let base = [224., 216., 196.][i] * (1.0 - f) + [111., 148., 88.][i] * f;
            c[i] = (base * gain * (1.0 - w) + [114., 172., 204.][i] * w)
                .clamp(0.0, 255.0)
                .round() as u8;
        }
        *p = Rgb(c);
    }
    Ok(out)
}
fn main() -> Result<()> {
    let arg = std::env::args_os()
        .nth(1)
        .context("比較結果のディレクトリを指定してください")?;
    let root = Path::new(&arg);
    let data: Value = serde_json::from_slice(&fs::read(root.join("results.json"))?)?;
    let rows = data["results"].as_array().context("測定結果がありません")?;
    if rows.first().is_some_and(|r| r.get("mode").is_some()) {
        return report_masks(root, rows);
    }
    let regions: Vec<_> = rows
        .iter()
        .filter(|r| r["layer"] == "forest" && r["distance"] == "0")
        .map(|r| r["region"].as_str().unwrap())
        .collect();
    ensure!(
        regions.len() == 5,
        "5地点の比較が完了してから実行してください"
    );
    let mut summary = Vec::new();
    for layer in ["forest", "water", "hillshade"] {
        let mut distances: Vec<&str> = rows
            .iter()
            .filter(|r| r["layer"] == layer)
            .map(|r| r["distance"].as_str().unwrap())
            .collect();
        distances.sort_by(|a, b| {
            a.parse::<f64>()
                .unwrap()
                .total_cmp(&b.parse::<f64>().unwrap())
        });
        distances.dedup();
        for distance in distances {
            let selected: Vec<_> = rows
                .iter()
                .filter(|r| r["layer"] == layer && r["distance"] == distance)
                .collect();
            ensure!(
                selected.len() == 5,
                "各品質の5地点分の生成が完了していません"
            );
            let sum = |key: &str| selected.iter().map(|r| number(r, key)).sum::<f64>();
            let rmse = |key: &str| {
                let errors: Vec<_> = selected
                    .iter()
                    .flat_map(|r| r[key].as_array().unwrap().iter())
                    .collect();
                (errors
                    .iter()
                    .map(|e| number(&e["error"], "rmse").powi(2))
                    .sum::<f64>()
                    / errors.len() as f64)
                    .sqrt()
            };
            let fine: Vec<_> = selected
                .iter()
                .map(|r| &r["stages"].as_array().unwrap().last().unwrap()["error_vs_current_level"])
                .collect();
            let grouped_rmse = (fine.iter().map(|e| number(e, "rmse").powi(2)).sum::<f64>()
                / fine.len() as f64)
                .sqrt();
            let flip = fine
                .iter()
                .map(|e| number(e, "half_threshold_flip_percent"))
                .sum::<f64>()
                / fine.len() as f64;
            let spill = fine
                .iter()
                .map(|e| number(e, "outside_zero_over_17_percent"))
                .sum::<f64>()
                / fine.len() as f64;
            summary.push(json!({"layer":layer,"distance":distance,"current_bytes":sum("current_bytes"),"independent_bytes":sum("independent_jxl_bytes"),"grouped_bytes":sum("grouped_jxl_bytes"),"grouped_nonprogressive_bytes":sum("grouped_nonprogressive_bytes"),"current_decode_ms":sum("current_decode_ms_21_tiles"),"independent_decode_ms":sum("independent_decode_ms_21_tiles"),"grouped_z12_rmse":grouped_rmse,"grouped_z12_half_flip_percent":flip,"grouped_z12_outside_over_17_percent":spill,"current_rmse":rmse("current_errors"),"independent_rmse":rmse("independent_errors")}));
        }
    }
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    let mut body = String::from(
        "<!doctype html><html lang=ja><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>地表 JPEG XL比較</title><style>body{font-family:system-ui,sans-serif;background:#f3f5f4;color:#25332d;max-width:1500px;margin:auto;padding:24px}table{border-collapse:collapse;width:100%;background:white;margin:16px 0}td,th{padding:8px;text-align:right;border-bottom:1px solid #ddd}td:first-child,th:first-child{text-align:left}img{width:100%;height:auto}figure{margin:0}summary{cursor:pointer;font-weight:bold;padding:12px}section{background:white;padding:16px;margin:16px 0}.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}select{font-size:16px;padding:8px}p{line-height:1.7}</style><h1>地表 JPEG XL比較</h1><p>5地点、各地点3レイヤー、各レイヤーz10を1枚・z11を4枚・z12を16枚。森林・水域は可逆とdistance 0.25 / 0.5 / 1 / 2、陰影は圧縮前PNGからの可逆とdistance 0.5 / 1。単位はKiB。測定はWindows PCのCPU・1スレッド、5回平均。GPU転送は含めない。容量はBLOB相当でSQLite索引・ページの増減は含めない。</p><table><tr><th>レイヤー / distance</th><th>現行</th><th>独立JXL</th><th>一括JXL（通常）</th><th>一括JXL（段階式）</th><th>現行RMSE</th><th>独立JXL RMSE</th><th>段階式z12 RMSE</th><th>50%濃度境界の反転率</th><th>元ゼロ領域の濃度&gt;17</th></tr>",
    );
    for row in &summary {
        body.push_str(&format!("<tr><td>{} / {}</td><td>{:.1}</td><td>{:.1}</td><td>{:.1}</td><td>{:.1}</td><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td><td>{:.3}%</td><td>{:.3}%</td></tr>",row["layer"].as_str().unwrap(),row["distance"].as_str().unwrap(),number(row,"current_bytes")/1024.,number(row,"independent_bytes")/1024.,number(row,"grouped_nonprogressive_bytes")/1024.,number(row,"grouped_bytes")/1024.,number(row,"current_rmse"),number(row,"independent_rmse"),number(row,"grouped_z12_rmse"),number(row,"grouped_z12_half_flip_percent"),number(row,"grouped_z12_outside_over_17_percent")));
    }
    let mut mask_distances: Vec<&str> = rows
        .iter()
        .filter(|r| r["layer"] == "forest")
        .map(|r| r["distance"].as_str().unwrap())
        .collect();
    mask_distances.sort_by(|a, b| {
        a.parse::<f64>()
            .unwrap()
            .total_cmp(&b.parse::<f64>().unwrap())
    });
    mask_distances.dedup();
    body.push_str("</table><p>一括版はz12の16枚を1536×1536にまとめ、その途中段階を粗いLODに利用する試作。粗い段階は現在のz10・z11の画像と完全一致するとは限らない。可逆版で1/4・1/2の復号イベントが得られなかった場合も、実際の比率1として記録する。境界の反転率は濃度128の判定が変わった画素の割合、元ゼロ領域の濃度&gt;17は元の濃度が0で復号後に17を超えた画素の割合。どちらも画像全体を分母とする。</p><label>表示レベル <select id=zoom><option>10</option><option>11</option><option selected>12</option></select></label> <label>森林・水域のdistance <select id=maskQuality>");
    for d in &mask_distances {
        body.push_str(&format!(
            "<option {}>{d}</option>",
            if *d == "1" { "selected" } else { "" }
        ));
    }
    body.push_str("</select></label><p>画像は森林・陰影・水域の合成。陰影は独立・段階式ともdistance 1固定で、森林・水域の品質を選択できる。distance 0は可逆。各画像をクリックすると原寸表示。</p>");
    for region in regions {
        body.push_str(&format!("<section><h2>{region}</h2><div class=grid>"));
        for kind in ["current", "independent", "grouped"] {
            let distances: Vec<_> = if kind == "current" {
                vec!["0"]
            } else {
                mask_distances.clone()
            };
            for d in distances {
                for z in 10..=12 {
                    let path = format!("{region}-{kind}-d{d}-z{z}.png");
                    color(root, region, z, kind, d)?.save(root.join(&path))?;
                }
            }
            body.push_str(&format!("<figure><figcaption>{}</figcaption><a class=compare data-region='{region}' data-kind='{kind}'><img loading=lazy alt='{region} {kind}'></a></figure>",match kind{"current"=>"現行WebP / PNG","independent"=>"独立JPEG XL",_=>"一括・段階的JPEG XL"}));
        }
        body.push_str("</div></section>");
    }
    body.push_str("<details><summary>段階別の読込量・復号時間・メモリ</summary><table><tr><th>地点 / レイヤー / d</th><th>z</th><th>実際の縮小比</th><th>必要入力KiB</th><th>復号ms</th><th>復号器＋出力MiB</th><th>粗レベルRMSE</th></tr>");
    for r in rows {
        for stage in r["stages"].as_array().unwrap() {
            body.push_str(&format!("<tr><td>{} / {} / {}</td><td>{}</td><td>1/{}</td><td>{:.1}</td><td>{:.2}</td><td>{:.2}</td><td>{:.2}</td></tr>",r["region"].as_str().unwrap(),r["layer"].as_str().unwrap(),r["distance"].as_str().unwrap(),stage["z"],stage["actual_ratio"],number(stage,"prefix_bytes")/1024.,number(stage,"decode_ms"),number(stage,"decoder_peak_plus_output_bytes")/1048576.,number(&stage["error_vs_current_level"],"rmse")));
        }
    }
    body.push_str("</table></details><p>メモリはlibjxlのカスタムアロケーターで記録したピーク＋8bitグレー出力バッファ。入力データ・画像リサイズ・GPUテクスチャ・プロセス全体を含まない。復号後の全解放と、区切り位置で入力を切っても同じ画像が復号できることを検証している。デコーダー実装差があるため、時間比を形式自体の性能と断定しない。</p><script>function update(){for(const a of document.querySelectorAll('.compare')){const d=a.dataset.kind==='current'?'0':document.getElementById('maskQuality').value;const p=a.dataset.region+'-'+a.dataset.kind+'-d'+d+'-z'+document.getElementById('zoom').value+'.png';a.href=p;a.querySelector('img').src=p}}document.getElementById('zoom').onchange=update;document.getElementById('maskQuality').onchange=update;update()</script></html>");
    fs::write(root.join("report.html"), body)?;
    println!("{}", root.join("report.html").display());
    Ok(())
}

// マスク単独で水際のにじみ・森林濃度を見比べる。陰影は重ねない。
fn report_masks(root: &Path, rows: &[Value]) -> Result<()> {
    let layers: Vec<&str> = rows
        .iter()
        .map(|r| r["layer"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    ensure!(
        rows.len() == layers.len() * 50,
        "5地点・対象レイヤー・2モード・5品質の完了後に実行してください"
    );
    let mut summary = Vec::new();
    for &layer in &layers {
        for mode in ["VarDCT", "Modular"] {
            for d in ["0.5", "1", "2", "4", "8"] {
                let set: Vec<_> = rows
                    .iter()
                    .filter(|r| r["layer"] == layer && r["mode"] == mode && r["distance"] == d)
                    .collect();
                ensure!(set.len() == 5, "地点不足");
                let sum = |key: &str| set.iter().map(|r| number(r, key)).sum::<f64>();
                let fine: Vec<_> = set.iter().map(|r| &r["stages"][2]["error"]).collect();
                summary.push(json!({"layer":layer,"mode":mode,"distance":d,"current_bytes":sum("current_bytes"),"grouped_bytes":sum("grouped_bytes"),"rmse":(fine.iter().map(|e|number(e,"rmse").powi(2)).sum::<f64>()/5.).sqrt(),"current_z12_rmse":(set.iter().map(|r|number(&r["current_z12_error"],"rmse").powi(2)).sum::<f64>()/5.).sqrt(),"mae":fine.iter().map(|e|number(e,"mae")).sum::<f64>()/5.,"half_flip_percent":fine.iter().map(|e|number(e,"half_threshold_flip_percent")).sum::<f64>()/5.,"outside_over_17_percent":fine.iter().map(|e|number(e,"outside_zero_over_17_percent")).sum::<f64>()/5.,"z10_prefix_bytes":set.iter().map(|r|number(&r["stages"][0],"prefix_bytes")).sum::<f64>(),"z10_decode_ms":set.iter().map(|r|number(&r["stages"][0],"decode_ms")).sum::<f64>(),"z10_actual_ratios":set.iter().map(|r|r["stages"][0]["actual_ratio"].clone()).collect::<Vec<_>>()}));
            }
        }
    }
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    for region in ["fuji", "tokyo", "izu", "yari", "tsukuba"] {
        for &layer in &layers {
            let dir = root.join(format!("{region}-{layer}"));
            for z in 10..=12 {
                for mode in ["current", "0", "1"] {
                    let distances = if mode == "current" {
                        vec!["0"]
                    } else {
                        vec!["0.5", "1", "2", "4", "8"]
                    };
                    for d in distances {
                        let input = if mode == "current" {
                            if dir.join(format!("current-z{z}.png")).exists() {
                                format!("current-z{z}.png")
                            } else {
                                format!("reference-z{z}.png")
                            }
                        } else {
                            format!("m{mode}-d{d}-z{z}.png")
                        };
                        let mask = image::open(dir.join(input))?.to_luma8();
                        let mut image = RgbImage::new(mask.width(), mask.height());
                        for (x, y, p) in image.enumerate_pixels_mut() {
                            if layer == "hillshade" {
                                *p = Rgb([mask[(x, y)][0]; 3]);
                                continue;
                            }
                            let alpha = mask[(x, y)][0] as f64 / 255.;
                            let color = if layer == "forest" {
                                [111., 148., 88.]
                            } else {
                                [114., 172., 204.]
                            };
                            *p = Rgb(std::array::from_fn(|i| {
                                ([224., 216., 196.][i] * (1. - alpha) + color[i] * alpha).round()
                                    as u8
                            }));
                        }
                        image.save(dir.join(format!("color-{mode}-d{d}-z{z}.png")))?;
                    }
                }
            }
        }
    }
    let mut html = String::from(
        "<!doctype html><html lang=ja><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>森林・水域 非可逆JPEG XL</title><style>body{font-family:system-ui,sans-serif;background:#f3f5f4;color:#24352e;max-width:1500px;margin:auto;padding:24px}p{line-height:1.7}table{width:100%;border-collapse:collapse;background:white}th,td{padding:7px;text-align:right;border-bottom:1px solid #ddd}th:first-child,td:first-child{text-align:left}.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}img{width:100%;height:auto}figure{margin:0}select{font-size:16px;padding:7px}nav{position:sticky;top:0;background:#f3f5f4;padding:14px 0}</style><h1>森林・水域の非可逆・段階式JPEG XL</h1><p>富士山・東京・伊豆・槍ヶ岳・筑波山の5地点。各地点のz12を16枚まとめて1536×1536に圧縮。現行欄は同じ範囲のz10・z11・z12の21枚合計。VarDCTとModularの両方を比較。VarDCTでは1/4・1/2で停止できた。Modularは今回のAPI設定でその区切りを得られず、粗レベルも原寸まで復号している。effort 7・1スレッド・5回平均。容量は画像BLOB相当。マスク単独で表示し、陰影は重ねていない。</p><nav><label>地点 <select id=region><option value=fuji>富士山</option><option value=tokyo>東京</option><option value=izu>伊豆</option><option value=yari>槍ヶ岳</option><option value=tsukuba>筑波山</option></select></label> <label>レイヤー <select id=layer><option value=forest>森林</option><option value=water>水域</option></select></label> <label>z <select id=zoom><option>10</option><option>11</option><option selected>12</option></select></label> <label>distance <select id=quality><option>0.5</option><option>1</option><option>2</option><option selected>4</option><option>8</option></select></label></nav><div class=grid><figure><figcaption>現行</figcaption><a data-mode=current><img></a></figure><figure><figcaption>非可逆・段階式 VarDCT</figcaption><a data-mode=0><img></a></figure><figure><figcaption>非可逆・段階式 Modular</figcaption><a data-mode=1><img></a></figure></div><p>クリックすると原寸画像。distanceは知覚的品質の設定で、誤差の上限ではない。VarDCTとModularで同じ値が同等画質を意味するとは限らない。粗レベルは途中出力をTriangleフィルターで縮小しており、現行の面積平均と完全一致しない。</p><table><tr><th>レイヤー / モード / distance</th><th>現行 KiB</th><th>段階式 KiB</th><th>増減</th><th>z12 MAE</th><th>z12 RMSE</th><th>50%境界反転率</th><th>元ゼロの濃度&gt;17</th></tr>",
    );
    for r in &summary {
        html.push_str(&format!("<tr><td>{} / {} / {}</td><td>{:.1}</td><td>{:.1}</td><td>{:+.1}%</td><td>{:.2}</td><td>{:.2}</td><td>{:.3}%</td><td>{:.3}%</td></tr>",r["layer"].as_str().unwrap(),r["mode"].as_str().unwrap(),r["distance"].as_str().unwrap(),number(r,"current_bytes")/1024.,number(r,"grouped_bytes")/1024.,100.*(number(r,"grouped_bytes")/number(r,"current_bytes")-1.),number(r,"mae"),number(r,"rmse"),number(r,"half_flip_percent"),number(r,"outside_over_17_percent")));
    }
    html.push_str("</table><p>濃度・誤差の単位は0〜255。境界反転率は元と復号後の128以上の判定が変わった画素の割合。元ゼロの濃度&gt;17は、本来ゼロだった領域で濃度が17を超えた割合。いずれも画像全体を分母とする。SQLite構造の変更・GPU・Android/iOS実機の計測は含めない。</p><script>function update(){const r=document.getElementById('region').value,l=document.getElementById('layer').value,z=document.getElementById('zoom').value,d=document.getElementById('quality').value;for(const a of document.querySelectorAll('a[data-mode]')){const m=a.dataset.mode,p=r+'-'+l+'/color-'+m+'-d'+(m==='current'?'0':d)+'-z'+z+'.png';a.href=p;a.querySelector('img').src=p}}for(const s of document.querySelectorAll('select'))s.onchange=update;update()</script></html>");
    if layers == ["hillshade"] {
        html=html.replace("森林・水域","陰影").replace("<option value=forest>森林</option><option value=water>水域</option>","<option value=hillshade>陰影</option>").replace("マスク単独で表示し、陰影は重ねていない。","陰影をグレースケールで表示。現行は配布済みWebP、JPEG XLは同じ圧縮前PNGから生成した。陰影の境界反転率は分類を意味しないため、RMSEと画像を参照する。");
    }
    fs::write(root.join("report.html"), html)?;
    println!("{}", root.join("report.html").display());
    Ok(())
}

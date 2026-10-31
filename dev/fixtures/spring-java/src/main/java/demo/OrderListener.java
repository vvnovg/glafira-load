package demo;

import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.kafka.core.KafkaTemplate;

public class OrderListener {
    private final KafkaTemplate<String, String> kafkaTemplate = null;

    @KafkaListener(topics = {"orders.created", "orders.paid"}, groupId = "audit")
    public void onOrder(String payload) {
        kafkaTemplate.send("orders.audit", payload);
    }
}
